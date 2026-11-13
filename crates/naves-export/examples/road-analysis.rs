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
    clippy::float_cmp,
    clippy::cast_possible_wrap,
    clippy::print_stderr,
    clippy::needless_range_loop,
    clippy::items_after_statements,
    reason = "a measuring tool for developers; failing loudly is the point"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use naves_core::checks::{
    DriveState, MAX_MISS_M, PieceCheck, check_roads_on_terrain, check_walkable,
};
use naves_core::conform::{FILL_SLOPE_MAX, MAX_EARTHWORK_M, conform};
use naves_core::dem::Dem;
use naves_core::features::road_features;
use naves_core::heightmap::{Heightmap, MapGeo, build_heightmap};
use naves_core::junctions::{HOLD_M, find_junctions};
use naves_core::limits::{CROSS_FALL_LIMIT, drive_grade, grade_aim, min_vertical_radius};
use naves_core::osm::read_ways;
use naves_core::profile::{RoadStrip, chain_holds, chain_holds_unchecked, solve_profiles};
use naves_core::project::Project;
use naves_core::roadnet::{build_graph, widen_bridges};
use naves_core::roadplace::{PlacedKind, bridge_for, place_network};
use naves_core::surface::LoadedSources;
use naves_core::validate::{Rule, check_roads};
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

struct Rec {
    slot: usize,
    cause: &'static str,
    len: f64,
    miss: f64,
    kink: f64,
    cap_d: f64,
}

struct Cf {
    slope: f64,
    len: f64,
    up: f64,
    mid: f64,
    down: f64,
    extent: Option<f64>,
}

fn dist_line(label: &str, mut v: Vec<f64>, thresholds: &[f64]) {
    let n = v.len();
    let (p50, p90, max) = (pct(&mut v, 0.5), pct(&mut v, 0.9), pct(&mut v, 1.0));
    let over: Vec<String> = thresholds
        .iter()
        .map(|t| format!(">{t}: {}", v.iter().filter(|x| **x > *t).count()))
        .collect();
    println!(
        "SIZE {label:32} n {n:5} p50 {p50:8.2} p90 {p90:8.2} max {max:8.2} | {}",
        over.join(", ")
    );
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
    let t0 = Instant::now();
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
    let t_dem = t0.elapsed();
    let base = build_heightmap(&project, &dem).expect("heightmap");
    let t_base = t0.elapsed();
    // shift study from a dump of the strips: skips the placing
    if let Ok(path) = std::env::var("SHIFT_LOAD") {
        let strips = load_strips(&path);
        if std::env::var("JUNCTION_ONLY").is_err() {
            shift_study(&strips, &base);
        }
        if std::env::var("JUNCTION_STUDY").is_ok() {
            match load_graph(&format!("{path}.graph")) {
                Some(g) => junction_study(&strips, &base, &g),
                None => eprintln!("no graph dump {path}.graph: re-dump with SHIFT_DUMP"),
            }
        }
        return;
    }
    let frame = MapGeo::new(&project).expect("frame");
    let window = frame.window(100.0).expect("window");
    let files: Vec<_> = project.sources.osm.iter().map(|p| dir.join(p)).collect();
    let ways = read_ways(&files, window).expect("ways");
    let features = road_features(&ways, &frame, project.map_size_m());
    let catalog = road_catalog(&tools, &tools.cache_dir).expect("catalog");
    let t_read = t0.elapsed();
    let built = build_graph(&features);
    let graph = widen_bridges(&built, &|s| {
        bridge_for(&catalog, s.family).map_or(0.0, |(m, scale, _)| m.length * scale)
    });
    {
        // OSM bridge stretches: before (built as a bridge only from 10 m chord) and after widening
        let chord = |s: &naves_core::roadnet::Stretch| match (s.points.first(), s.points.last()) {
            (Some(a), Some(b)) => (b.0 - a.0).hypot(b.1 - a.1),
            _ => 0.0,
        };
        let osm: Vec<_> = built
            .stretches
            .iter()
            .zip(&graph.stretches)
            .filter(|(b, _)| b.bridge && !b.tunnel)
            .collect();
        let before = osm.iter().filter(|(b, _)| chord(b) >= 10.0).count();
        let after = osm.iter().filter(|(_, a)| a.bridge).count();
        let plen = |s: &naves_core::roadnet::Stretch| -> f64 {
            s.points
                .windows(2)
                .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
                .sum()
        };
        let mut grown: Vec<f64> = osm
            .iter()
            .filter(|(b, a)| a.bridge && plen(a) - plen(b) > 1e-6)
            .map(|(b, a)| plen(a) - plen(b))
            .collect();
        println!(
            "OSMBRIDGE stretches {} built as bridge before {before} after {after} widened {}",
            osm.len(),
            grown.len()
        );
        if !grown.is_empty() {
            println!(
                "OSMBRIDGE widened by p10/p50/p90/max {:.1} {:.1} {:.1} {:.1} m",
                pct(&mut grown, 0.1),
                pct(&mut grown, 0.5),
                pct(&mut grown, 0.9),
                pct(&mut grown, 1.0)
            );
        }
        for (b, _) in osm.iter().filter(|(b, a)| b.bridge && !a.bridge) {
            let m = b
                .points
                .get(b.points.len() / 2)
                .copied()
                .unwrap_or_default();
            println!(
                "OSMBRIDGE no room {} length {:.1} m at ({:.0}, {:.0})",
                b.id,
                plen(b),
                m.0,
                m.1
            );
        }
    }
    let net = place_network(&graph, &catalog).expect("place");
    let t_place = t0.elapsed();
    let junctions = find_junctions(&net, &graph, &base);
    let side = naves_core::sideslope::side_slopes(&net, &base);
    let profiles = if std::env::var("NO_JUNCTIONS").is_ok() {
        solve_profiles(
            &net,
            &base,
            &naves_core::junctions::Junctions::default(),
            &side,
        )
    } else {
        solve_profiles(&net, &base, &junctions, &side)
    };
    let t_profiles = t0.elapsed();
    if let Ok(path) = std::env::var("SHIFT_DUMP") {
        dump_strips(&path, &profiles.strips, &net);
        dump_graph(&format!("{path}.graph"), &graph, &net);
        if std::env::var("SHIFT_ONLY").is_ok() {
            return;
        }
    }
    let patches = naves_export::generate::level_patches(&junctions, f64::from(base.cell_m));
    let conformed = conform(&base, &profiles.strips, &patches).expect("conform");
    let t_conform = t0.elapsed();
    let pieces = check_roads_on_terrain(&net, &conformed.heightmap, &base, &side);
    let raw = check_roads_on_terrain(&net, &base, &base, &side);
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
            // what the ground between two legs can take: nothing within 3 cells, then the step a cell holds
            let room = if c < margin {
                0.1
            } else {
                (c - margin + cell).max(0.0) * FILL_SLOPE_MAX + 0.1
            };
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
        .filter(|e| e.kind == naves_core::profile::GradeExcessKind::Grade)
        .map(|e| e.grade.abs())
        .collect();
    println!(
        "grade excesses (grade, not change): {} p50 {:.2} p90 {:.2} max {:.2}",
        ex.len(),
        pct(&mut ex, 0.5),
        pct(&mut ex, 0.9),
        pct(&mut ex, 1.0)
    );

    // =====================================================================================================
    // Additions of the grid-size measurement: violations, causes of misses, cut and fill, extremes, cost.
    // =====================================================================================================
    println!(
        "\n#### SECTION: violations as the build counts them (size {})",
        base.size
    );
    println!(
        "TIMING open DEM {:.1} s, heightmap {:.1} s, read OSM+catalog {:.1} s, placement {:.1} s, profiles {:.1} s, conform {:.1} s (cumulative at the end of the profiles: {:.1} s)",
        t_dem.as_secs_f64(),
        t_base.as_secs_f64() - t_dem.as_secs_f64(),
        t_read.as_secs_f64() - t_base.as_secs_f64(),
        t_place.as_secs_f64() - t_read.as_secs_f64(),
        t_profiles.as_secs_f64() - t_place.as_secs_f64(),
        t_conform.as_secs_f64() - t_profiles.as_secs_f64(),
        t_profiles.as_secs_f64()
    );
    let sources = LoadedSources::open(&project, dir, &|_| {}).expect("sources");
    let water = sources
        .osm
        .as_ref()
        .map_or(&[][..], |o| o.polygons.as_slice());
    let steep_paths = check_walkable(&features, hm);
    let violations = check_roads(
        &net,
        &profiles,
        &junctions,
        &conformed,
        &pieces,
        &steep_paths,
        &[],
        water,
        &frame,
    );
    println!("violations total {}", violations.len());
    {
        // bridge objects: by model, scale distribution, width warnings, wooden against paved by family
        let mut by_model: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut by_family: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut scales: BTreeMap<String, usize> = BTreeMap::new();
        for r in net.roads.iter().filter(|r| r.kind == PlacedKind::Bridge) {
            let scaled = r.scale > 1.0 + 1e-9;
            let e = by_model.entry(r.model.clone()).or_default();
            e.0 += 1;
            e.1 += usize::from(scaled);
            let f = by_family.entry(format!("{:?}", r.family)).or_default();
            if r.model.contains("wood") {
                f.0 += 1;
            } else {
                f.1 += 1;
            }
            if scaled {
                *scales.entry(format!("{:.2}", r.scale)).or_default() += 1;
            }
        }
        for (m, (n, sc)) in &by_model {
            println!("BRIDGE model {m}: {n} objects, {sc} scaled");
        }
        for (s, n) in &scales {
            println!("BRIDGE scale {s}: {n}");
        }
        for (f, (w, p)) in &by_family {
            println!("BRIDGE family {f}: wooden {w}, paved {p}");
        }
        let width_warnings = net
            .warnings
            .iter()
            .filter(|w| w.starts_with("bridge on"))
            .count();
        println!("BRIDGE width warnings {width_warnings}");
    }
    for rule in Rule::ALL {
        let n = violations.iter().filter(|v| v.rule == rule).count();
        println!("RULE {:24} {n}", rule.id());
    }
    // roads far from the ground: by family, the extremes and the distribution of |off_ground_m|
    {
        println!("off ground: family, pieces, over the allowance, p50/p90/max |off_ground_m|");
        let mut by_family: BTreeMap<String, Vec<&PieceCheck>> = BTreeMap::new();
        for p in pieces.iter().filter(|p| !p.off_ground_m.is_nan()) {
            by_family
                .entry(format!("{:?}", p.family))
                .or_default()
                .push(p);
        }
        for (family, list) in &by_family {
            let over = list
                .iter()
                .filter(|p| {
                    p.off_ground_m.abs() > naves_core::limits::earthwork_allowance_m(p.side_slope)
                })
                .count();
            let mut abs: Vec<f64> = list.iter().map(|p| p.off_ground_m.abs()).collect();
            println!(
                "  OFFGROUND {family:5} {:6} {over:4}   {:.1}/{:.1}/{:.1}",
                list.len(),
                pct(&mut abs, 0.5),
                pct(&mut abs, 0.9),
                pct(&mut abs, 1.0)
            );
        }
        let by_height =
            |a: &&PieceCheck, b: &&PieceCheck| a.off_ground_m.total_cmp(&b.off_ground_m);
        let all: Vec<&PieceCheck> = pieces.iter().filter(|p| !p.off_ground_m.is_nan()).collect();
        if let (Some(cut), Some(bank)) = (
            all.iter().copied().min_by(by_height),
            all.iter().copied().max_by(by_height),
        ) {
            for (what, p) in [("deepest cut", cut), ("highest embankment", bank)] {
                println!(
                    "  OFFGROUND {what}: {:.1} m, side slope {:.2}, allowance {:.0} m, at ({:.0},{:.0}), {:?}",
                    p.off_ground_m,
                    p.side_slope,
                    naves_core::limits::earthwork_allowance_m(p.side_slope),
                    f64::midpoint(p.start.0, p.end.0),
                    f64::midpoint(p.start.1, p.end.1),
                    p.family
                );
            }
        }
        for v in violations
            .iter()
            .filter(|v| v.rule == Rule::RoadFarFromGround)
        {
            println!(
                "  OFFGROUND violation at ({:.0},{:.0}): {}",
                v.at.0, v.at.1, v.message
            );
        }
    }
    println!("sizes (m, % or degrees as named); '>t' counts values above t");
    dist_line(
        "road_misses_terrain miss m",
        pieces
            .iter()
            .filter(|p| p.miss_m > MAX_MISS_M)
            .map(|p| p.miss_m)
            .collect(),
        &[0.5, 1.0, 2.0, 5.0],
    );
    dist_line(
        "road_conflict m",
        conformed
            .conflicts
            .iter()
            .map(|c| c.height_difference_m)
            .collect(),
        &[1.0, 2.0, 5.0, 10.0],
    );
    dist_line(
        "road_gap m",
        net.gaps.iter().map(|g| g.gap_m).collect(),
        &[1.0, 2.0, 5.0, 10.0],
    );
    dist_line(
        "road_grade (grade) %",
        profiles
            .excesses
            .iter()
            .filter(|e| e.kind == naves_core::profile::GradeExcessKind::Grade)
            .map(|e| e.grade.abs() * 100.0)
            .collect(),
        &[35.0, 50.0, 100.0, 200.0],
    );
    dist_line(
        "road_grade (vertical radius) m",
        profiles
            .excesses
            .iter()
            .filter(|e| e.kind == naves_core::profile::GradeExcessKind::VerticalBend)
            .map(|e| 1.0 / e.grade.abs().max(1e-9))
            .collect(),
        &[],
    );
    // the not-drivable runs: numbers are in the message
    let num_after = |text: &str, key: &str| -> Option<f64> {
        let rest = text.split(key).nth(1)?;
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        rest[..end].parse().ok()
    };
    let nd: Vec<&naves_core::validate::Violation> = violations
        .iter()
        .filter(|v| v.rule == Rule::RoadNotDrivable)
        .collect();
    dist_line(
        "road_not_drivable grade %",
        nd.iter()
            .filter_map(|v| num_after(&v.message, "Not drivable: "))
            .collect(),
        &[50.0, 100.0, 200.0],
    );
    dist_line(
        "road_not_drivable length m",
        nd.iter()
            .filter_map(|v| num_after(&v.message, " % over "))
            .collect(),
        &[25.0, 100.0],
    );
    dist_line(
        "road_not_drivable cross fall %",
        nd.iter()
            .filter_map(|v| num_after(&v.message, "(and "))
            .collect(),
        &[30.0, 50.0, 100.0],
    );
    dist_line(
        "path_too_steep deg",
        steep_paths.iter().map(|s| s.slope_deg).collect(),
        &[55.0, 60.0, 70.0],
    );
    let open: BTreeSet<&str> = net.gaps.iter().map(|g| g.stretch.as_str()).collect();
    println!(
        "STATS pieces {} checked_pieces {} stretches {} open {} closed {} not_drivable_m {:.0} max_cut_m {:.1} max_fill_m {:.1} largest_miss_m {:.2} max_grade {:.2} capped_vertices {}",
        net.roads.len(),
        pieces.len(),
        graph.stretches.len(),
        open.len(),
        graph.stretches.len().saturating_sub(open.len()),
        pieces
            .iter()
            .filter(|p| p.state == DriveState::NotDrivable)
            .map(len)
            .sum::<f64>(),
        conformed.max_cut_m,
        conformed.max_fill_m,
        pieces.iter().map(|p| p.miss_m).fold(0.0, f64::max),
        pieces.iter().map(|p| p.grade).fold(0.0, f64::max),
        conformed.capped.len()
    );

    // ---- causes of misses
    println!("\n#### SECTION: causes of misses (one cause per piece, tested in the order printed)");
    let edge_m = 3.0 * cell;
    let dh_limit = 0.3;
    let kink_limit = 1.0;
    let map_m = project.map_size_m();
    let cap_reach = MAX_EARTHWORK_M + 2.0 * cell;
    println!(
        "(the two margins for 'other road' are printed per variant below) thresholds: bridge = chain neighbour is a bridge piece, or the piece's chord within one piece length of a bridge piece's end; map edge = an end within {edge_m:.1} m (3 cells) of the border; other road = edge-to-edge clearance below the variant's margin (3 cells, or a fixed 10 m) at one of 5 samples, planned height difference > {dh_limit} m (different) else same; hairpin = same rule for another part of the same strip, more than 1.4 d + 3 m of road away; capped = a capped vertex within {cap_reach:.1} m (60 m + 2 cells) of the chord; kink = change of planned grade at one of the piece's own joints > {kink_limit} %/m; bend = heading change over the piece > 0.001 rad"
    );
    let bridge_ends: Vec<(f64, f64)> = net
        .roads
        .iter()
        .filter(|r| r.kind == PlacedKind::Bridge)
        .flat_map(|r| [(r.start.x, r.start.y), (r.end.x, r.end.y)])
        .collect();
    let mut bridge_next: BTreeSet<usize> = BTreeSet::new();
    for chain in &net.chains {
        for (i, &slot) in chain.iter().enumerate() {
            let is_bridge = |j: Option<usize>| {
                j.and_then(|j| chain.get(j))
                    .is_some_and(|&s| net.roads[s].kind == PlacedKind::Bridge)
            };
            if is_bridge(i.checked_sub(1)) || is_bridge(Some(i + 1)) {
                bridge_next.insert(slot);
            }
        }
    }
    let mut cap_grid: BTreeMap<(i64, i64), Vec<(f64, f64)>> = BTreeMap::new();
    for c in &conformed.capped {
        cap_grid
            .entry((bucket(c.0), bucket(c.1)))
            .or_default()
            .push(*c);
    }
    let mut recs_main: Vec<Rec> = Vec::new();
    let mut recs_strict: Vec<Rec> = Vec::new();
    for (variant, near) in [("3 cells", 3.0 * cell), ("fixed 10 m", 10.0)] {
        println!("--- variant: other-road/hairpin margin {variant} = {near:.1} m edge to edge");
        let mut recs: Vec<Rec> = Vec::new();
        let mut no_segment = 0usize;
        // pieces missing by more than 0.5 m whose cause is another road at a different height: all, with a ring
        // vertex given to another core that is also within one cell of the other road's width (contested), with
        // such a vertex that is not (taken without need), with only the latter
        let mut ring = [0usize; 5];
        for p in pieces.iter().filter(|p| p.miss_m > MAX_MISS_M) {
            let Some(&own) = slot_seg.get(&p.slot) else {
                no_segment += 1;
                continue;
            };
            let s = &segs[own];
            let length = len(p).max(0.5);
            let mid = ((s.a.0 + s.b.0) / 2.0, (s.a.1 + s.b.1) / 2.0);
            let (mut other_diff, mut other_same, mut self_diff) = (false, false, false);
            for gx in bucket(mid.0) - 1..=bucket(mid.0) + 1 {
                for gy in bucket(mid.1) - 1..=bucket(mid.1) + 1 {
                    for &j in grid.get(&(gx, gy)).map_or(&[][..], Vec::as_slice) {
                        let o = &segs[j];
                        if j == own || o.bridge {
                            continue;
                        }
                        for k in 0..=4 {
                            let t = f64::from(k) / 4.0;
                            let q = (s.a.0 + t * (s.b.0 - s.a.0), s.a.1 + t * (s.b.1 - s.a.1));
                            let qh = s.a.2 + t * (s.b.2 - s.a.2);
                            let (d, u) = seg_dist(q, (o.a.0, o.a.1), (o.b.0, o.b.1));
                            let oh = o.a.2 + u * (o.b.2 - o.a.2);
                            let clear = d - strips[s.strip].half_width - strips[o.strip].half_width;
                            if clear >= near {
                                continue;
                            }
                            let different = (qh - oh).abs() > dh_limit;
                            if o.strip == s.strip {
                                let run = (o.along + u * (o.b.0 - o.a.0).hypot(o.b.1 - o.a.1)
                                    - (s.along + t * length))
                                    .abs();
                                if run <= 1.4 * d + 3.0 || o.index.abs_diff(s.index) <= 1 {
                                    continue;
                                }
                                self_diff |= different;
                            } else if different {
                                other_diff = true;
                            } else {
                                other_same = true;
                            }
                        }
                    }
                }
            }
            // nearest capped vertex
            let mut cap_d = f64::INFINITY;
            for gx in bucket(mid.0) - 2..=bucket(mid.0) + 2 {
                for gy in bucket(mid.1) - 2..=bucket(mid.1) + 2 {
                    for c in cap_grid.get(&(gx, gy)).map_or(&[][..], Vec::as_slice) {
                        cap_d = cap_d.min(seg_dist(*c, p.start, p.end).0);
                    }
                }
            }
            // change of planned grade at the piece's own joints, %/m
            let grade_of =
                |g: &Seg| (g.b.2 - g.a.2) / (g.b.0 - g.a.0).hypot(g.b.1 - g.a.1).max(0.5);
            let seg_len = |g: &Seg| (g.b.0 - g.a.0).hypot(g.b.1 - g.a.1).max(0.5);
            let mut kink = 0.0_f64;
            if s.index > 0 && own > 0 && segs[own - 1].strip == s.strip {
                let o = &segs[own - 1];
                kink = kink.max(
                    (grade_of(s) - grade_of(o)).abs() / ((seg_len(s) + seg_len(o)) / 2.0) * 100.0,
                );
            }
            if own + 1 < segs.len() && segs[own + 1].strip == s.strip {
                let o = &segs[own + 1];
                kink = kink.max(
                    (grade_of(s) - grade_of(o)).abs() / ((seg_len(s) + seg_len(o)) / 2.0) * 100.0,
                );
            }
            let road = &net.roads[p.slot];
            let turn = (road.end.heading - road.start.heading + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;
            let at_edge = |q: (f64, f64)| {
                q.0 < edge_m || q.1 < edge_m || q.0 > map_m - edge_m || q.1 > map_m - edge_m
            };
            let cause = if bridge_next.contains(&p.slot)
                || bridge_ends
                    .iter()
                    .any(|e| seg_dist(*e, p.start, p.end).0 <= length)
            {
                "1 bridge end"
            } else if at_edge(p.start) || at_edge(p.end) {
                "2 map edge"
            } else if other_diff {
                "3 other road, different height"
            } else if self_diff {
                "4 same road (hairpin leg), different height"
            } else if other_same {
                "5 other road, same height (overlap/junction)"
            } else if cap_d <= cap_reach {
                "6 earthworks capped near"
            } else if kink > kink_limit {
                "7 grade change at own joint"
            } else if turn.abs() > 0.001 {
                "8 bend (arc)"
            } else {
                "9 unknown"
            };
            if cause.starts_with("3 ") && p.miss_m > 0.5 {
                let hw = strips[s.strip].half_width;
                let reach = hw + cell;
                let (mut contested, mut needless, mut by_patch) = (false, false, false);
                let c0 = ((s.a.0.min(s.b.0) - reach) / cell).floor().max(0.0) as u32;
                let c1 = ((s.a.0.max(s.b.0) + reach) / cell).ceil().max(0.0) as u32;
                let r0 = ((s.a.1.min(s.b.1) - reach) / cell).floor().max(0.0) as u32;
                let r1 = ((s.a.1.max(s.b.1) + reach) / cell).ceil().max(0.0) as u32;
                for row in r0..=r1.min(hm.size - 1) {
                    for col in c0..=c1.min(hm.size - 1) {
                        let v = (f64::from(col) * cell, f64::from(row) * cell);
                        let (d, t) = seg_dist(v, (s.a.0, s.a.1), (s.b.0, s.b.1));
                        if d > reach {
                            continue;
                        }
                        let own_h = s.a.2 + t * (s.b.2 - s.a.2);
                        if (f64::from(hm.at(col, row)) - own_h).abs() <= dh_limit {
                            continue;
                        }
                        // not taken: the strip itself has this height there (a neighbouring piece of the same
                        // strip, a bend)
                        let mut own_strip = false;
                        let mut near_other = false;
                        for gx in bucket(v.0) - 1..=bucket(v.0) + 1 {
                            for gy in bucket(v.1) - 1..=bucket(v.1) + 1 {
                                for &j in grid.get(&(gx, gy)).map_or(&[][..], Vec::as_slice) {
                                    let o = &segs[j];
                                    if o.bridge {
                                        continue;
                                    }
                                    let (od, u) = seg_dist(v, (o.a.0, o.a.1), (o.b.0, o.b.1));
                                    let oh = o.a.2 + u * (o.b.2 - o.a.2);
                                    if o.strip == s.strip {
                                        own_strip |= od <= hw + 1.5 * cell
                                            && (f64::from(hm.at(col, row)) - oh).abs() <= dh_limit;
                                    } else if od <= strips[o.strip].half_width + cell
                                        && (oh - own_h).abs() > dh_limit
                                    {
                                        near_other = true;
                                    }
                                }
                            }
                        }
                        if own_strip {
                            continue;
                        }
                        if near_other {
                            contested = true;
                        } else {
                            needless = true;
                            if patches
                                .iter()
                                .any(|q| (q.at.0 - v.0).hypot(q.at.1 - v.1) <= q.radius)
                            {
                                by_patch = true;
                            }
                        }
                    }
                }
                ring[0] += 1;
                ring[1] += usize::from(contested);
                ring[2] += usize::from(needless);
                ring[3] += usize::from(needless && !contested);
                ring[4] += usize::from(needless && by_patch);
            }
            recs.push(Rec {
                slot: p.slot,
                cause,
                len: length,
                miss: p.miss_m,
                kink,
                cap_d,
            });
        }
        println!(
            "RINGTAKEN {variant}: pieces missing > 0.5 m by another road at a different height {} | with a vertex within one cell of their width given to another core that is also within one cell of the other road's width {} | with such a vertex that is not (taken without need) {} | with only such vertices {} | of the needless, inside a level patch {}",
            ring[0], ring[1], ring[2], ring[3], ring[4]
        );
        println!("pieces with a miss but no profile segment (skipped): {no_segment}");
        for (title, lo, hi) in [
            ("MISS > 0.5 m", 0.5, f64::MAX),
            ("MISS 0.15 to 0.5 m", MAX_MISS_M, 0.5),
        ] {
            println!(
                "{title} [{variant}]: cause | pieces | km | miss p50 | miss p90 | kink p50 %/m | share with capped vertex within 3 cells"
            );
            let mut causes_here: BTreeMap<&str, Vec<&Rec>> = BTreeMap::new();
            for r in recs.iter().filter(|r| r.miss > lo && r.miss <= hi) {
                causes_here.entry(r.cause).or_default().push(r);
            }
            let mut total = 0usize;
            for (cause, list) in &causes_here {
                total += list.len();
                let mut m: Vec<f64> = list.iter().map(|r| r.miss).collect();
                let mut k: Vec<f64> = list.iter().map(|r| r.kink).collect();
                println!(
                    "CAUSE {variant:10} {title:20} | {cause:46} | {:5} | {:7.3} | {:6.2} | {:6.2} | {:6.2} | {:.2}",
                    list.len(),
                    list.iter().map(|r| r.len).sum::<f64>() / 1000.0,
                    pct(&mut m, 0.5),
                    pct(&mut m, 0.9),
                    pct(&mut k, 0.5),
                    list.iter().filter(|r| r.cap_d <= near + 3.0).count() as f64
                        / list.len() as f64
                );
            }
            println!("  total {total}");
        }
        if variant == "3 cells" {
            recs_main = recs;
        } else {
            recs_strict = recs;
        }
    }
    let by_slot: BTreeMap<usize, &PieceCheck> = pieces.iter().map(|p| (p.slot, p)).collect();
    let detail = |slot: usize| {
        let p = by_slot[&slot];
        let own_seg = slot_seg[&slot];
        let s = &segs[own_seg];
        let road = &net.roads[slot];
        let length = len(p).max(0.5);
        let dir = (
            (p.end.0 - p.start.0) / length,
            (p.end.1 - p.start.1) / length,
        );
        let right = (dir.1, -dir.0);
        let hw = p.width / 2.0;
        println!(
            "DETAIL slot {slot} stretch {} index {} {:?} {:?} model {} width {:.1} len {:.2} miss {:.2}",
            road.slot.stretch,
            road.slot.index,
            road.family,
            road.kind,
            road.model.rsplit('\\').next().unwrap_or(""),
            p.width,
            length,
            p.miss_m
        );
        let neighbour = |i: Option<usize>| {
            i.and_then(|i| segs.get(i))
                .filter(|o| o.strip == s.strip)
                .map_or_else(
                    || "-".to_string(),
                    |o| {
                        format!(
                            "{:.1}%",
                            (o.b.2 - o.a.2) / (o.b.0 - o.a.0).hypot(o.b.1 - o.a.1).max(0.5) * 100.0
                        )
                    },
                )
        };
        let own = slot_seg[&slot];
        println!(
            "  planned height {:.2} -> {:.2}, planned grade {:.1}%, previous segment {}, next segment {}; terrain at ends {:.2} -> {:.2}; base at ends {:.2} -> {:.2}",
            s.a.2,
            s.b.2,
            (s.b.2 - s.a.2) / length * 100.0,
            neighbour(own.checked_sub(1)),
            neighbour(Some(own + 1)),
            hm.sample(s.a.0, s.a.1).height,
            hm.sample(s.b.0, s.b.1).height,
            base.sample(s.a.0, s.a.1).height,
            base.sample(s.b.0, s.b.1).height
        );
        let corner = |u: f64, v: f64| {
            let q = (
                p.start.0 + dir.0 * u + right.0 * v,
                p.start.1 + dir.1 * u + right.1 * v,
            );
            f64::from(hm.sample(q.0, q.1).height)
        };
        let corners = (
            corner(0.0, -hw),
            corner(0.0, hw),
            corner(length, -hw),
            corner(length, hw),
        );
        println!(
            "  quad corners (terrain): near-left {:.2} near-right {:.2} far-left {:.2} far-right {:.2}",
            corners.0, corners.1, corners.2, corners.3
        );
        println!(
            "  grid vertices within one cell of the footprint: along across | terrain | quad plane | terrain-plane | planned at along | base"
        );
        let (c0, c1) = (
            ((p.start.0.min(p.end.0) - hw - cell) / cell)
                .floor()
                .max(0.0) as u32,
            ((p.start.0.max(p.end.0) + hw + cell) / cell).ceil() as u32,
        );
        let (r0, r1) = (
            ((p.start.1.min(p.end.1) - hw - cell) / cell)
                .floor()
                .max(0.0) as u32,
            ((p.start.1.max(p.end.1) + hw + cell) / cell).ceil() as u32,
        );
        for r in r0..=r1 {
            for c in c0..=c1 {
                let (x, y) = (f64::from(c) * cell, f64::from(r) * cell);
                let (u, v) = (
                    (x - p.start.0) * dir.0 + (y - p.start.1) * dir.1,
                    (x - p.start.0) * right.0 + (y - p.start.1) * right.1,
                );
                if u < -cell || u > length + cell || v.abs() > hw + cell {
                    continue;
                }
                let (tu, tv) = (
                    (u / length).clamp(0.0, 1.0),
                    ((v + hw) / (2.0 * hw)).clamp(0.0, 1.0),
                );
                let plane = corners.0 * (1.0 - tu) * (1.0 - tv)
                    + corners.1 * (1.0 - tu) * tv
                    + corners.2 * tu * (1.0 - tv)
                    + corners.3 * tu * tv;
                let terrain = f64::from(hm.at(c, r));
                let planned = s.a.2 + (u / length).clamp(0.0, 1.0) * (s.b.2 - s.a.2);
                let inside = u >= 0.0 && u <= length && v.abs() <= hw;
                println!(
                    "    {} {u:6.2} {v:6.2} | {terrain:8.2} | {plane:8.2} | {:6.2} | {planned:8.2} | {:8.2}",
                    if inside { "in " } else { "out" },
                    terrain - plane,
                    base.at(c, r)
                );
                if (terrain - planned).abs() > 0.5 {
                    // who holds this vertex: the closest part of each of the nearest roads
                    let mut near_segs: Vec<(f64, f64, usize)> = segs
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != own_seg)
                        .map(|(i, g)| {
                            let (d, t) = seg_dist((x, y), (g.a.0, g.a.1), (g.b.0, g.b.1));
                            (d, t, i)
                        })
                        .filter(|(d, _, _)| *d < 40.0)
                        .collect();
                    near_segs.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let mut seen_strips: Vec<usize> = Vec::new();
                    println!(
                        "        vertex is {:.2} m from planned; nearest road parts (core reach = half width + 1.5 cells):",
                        terrain - planned
                    );
                    for (d, t, i) in near_segs {
                        let g = &segs[i];
                        if seen_strips.contains(&g.strip) {
                            continue;
                        }
                        seen_strips.push(g.strip);
                        if seen_strips.len() > 4 {
                            break;
                        }
                        println!(
                            "          strip {} (own strip: {}, rank {}, bridge {}): {d:.1} m away, core reaches {:.1} m, planned height there {:.2}",
                            g.strip,
                            g.strip == s.strip,
                            strips[g.strip].rank,
                            g.bridge,
                            strips[g.strip].half_width + 1.5 * cell,
                            g.a.2 + t * (g.b.2 - g.a.2)
                        );
                    }
                }
            }
        }
    };
    for (variant, recs) in [("3 cells", &recs_main), ("fixed 10 m", &recs_strict)] {
        let mut unknown: Vec<&Rec> = recs
            .iter()
            .filter(|r| r.cause == "9 unknown" && r.miss > 0.5)
            .collect();
        unknown.sort_by(|a, b| b.miss.total_cmp(&a.miss));
        println!(
            "UNKNOWN group [{variant} margin] (miss > 0.5 m): {} pieces; ten examples spread over the group by miss (largest first)",
            unknown.len()
        );
        let picks: Vec<&Rec> = if unknown.len() <= 10 {
            unknown.clone()
        } else {
            (0..10)
                .map(|i| unknown[i * (unknown.len() - 1) / 9])
                .collect()
        };
        for r in &picks {
            let p = by_slot[&r.slot];
            let road = &net.roads[r.slot];
            println!(
                "UNKNOWN_EX [{variant}] slot {} ({:.1},{:.1})-({:.1},{:.1}) {:?} {:?} {} len {:.1} miss {:.2} grade {:.1}% fall {:.1}% kink {:.2}%/m capped_d {:.0}",
                r.slot,
                p.start.0,
                p.start.1,
                p.end.0,
                p.end.1,
                road.family,
                road.kind,
                road.model.rsplit('\\').next().unwrap_or(""),
                r.len,
                r.miss,
                p.grade * 100.0,
                p.cross_fall * 100.0,
                r.kink,
                if r.cap_d.is_finite() { r.cap_d } else { -1.0 }
            );
        }
        let mut detail_slots: Vec<usize> = Vec::new();
        if !picks.is_empty() {
            for i in [0usize, picks.len() / 2, picks.len() * 3 / 4] {
                let slot = picks[i.min(picks.len() - 1)].slot;
                if !detail_slots.contains(&slot) {
                    detail_slots.push(slot);
                }
            }
        }
        for slot in &detail_slots {
            println!("DETAIL of an unknown piece [{variant} margin]");
            detail(*slot);
        }
    }

    // ---- cut against fill by side slope of the original ground
    println!("\n#### SECTION: cut and fill by the side slope of the original ground");
    println!(
        "side slope = |base(+10 m) - base(-10 m)| / 20 m across the piece, mean of 3 stations; surface = finished terrain at the road edges/centre; delta = finished minus original (negative: the road is cut into the ground); fill extent = distance from the downhill edge to where finished minus original first falls below 0.25 m, counted only where it is above 0.25 m at the edge (not met within 70 m counts as 70)"
    );
    let mut cfs: Vec<Cf> = Vec::new();
    let at_h = |m: &Heightmap, q: (f64, f64)| f64::from(m.sample(q.0, q.1).height);
    for p in pieces.iter().filter(|p| net.roads[p.slot].drivable) {
        let length = len(p);
        if length < 0.5 {
            continue;
        }
        let dir = (
            (p.end.0 - p.start.0) / length,
            (p.end.1 - p.start.1) / length,
        );
        let right = (dir.1, -dir.0);
        let hw = p.width / 2.0;
        let (mut slope, mut up, mut mid, mut down, mut extent) = (0.0, 0.0, 0.0, 0.0, None::<f64>);
        let mut inside = true;
        let stations = [0.15, 0.5, 0.85];
        for t in stations {
            let c = (
                p.start.0 + dir.0 * length * t,
                p.start.1 + dir.1 * length * t,
            );
            let off = |d: f64| (c.0 + right.0 * d, c.1 + right.1 * d);
            let (hr, hl) = (at_h(&base, off(10.0)), at_h(&base, off(-10.0)));
            let sign = if hr >= hl { 1.0 } else { -1.0 };
            slope += (hr - hl).abs() / 20.0 / 3.0;
            let delta = |q: (f64, f64)| at_h(hm, q) - at_h(&base, q);
            up += delta(off(sign * hw)) / 3.0;
            mid += delta(c) / 3.0;
            down += delta(off(-sign * hw)) / 3.0;
            if delta(off(-sign * hw)) > 0.25 {
                let mut found = 70.0;
                for k in 0..=70 {
                    let q = off(-sign * (hw + f64::from(k)));
                    if q.0 < 0.0 || q.1 < 0.0 || q.0 > map_m || q.1 > map_m {
                        inside = false;
                        break;
                    }
                    if delta(q) < 0.25 {
                        found = f64::from(k);
                        break;
                    }
                }
                extent = Some(extent.map_or(found, |e: f64| e.max(found)));
            }
        }
        if inside {
            cfs.push(Cf {
                slope,
                len: length,
                up,
                mid,
                down,
                extent,
            });
        }
    }
    let bands = [
        (0.0, 0.10),
        (0.10, 0.20),
        (0.20, 0.35),
        (0.35, 0.60),
        (0.60, 99.0),
    ];
    println!(
        "CF band | pieces | km | uphill edge p50 p90 | centre p50 p90 | downhill edge p50 p90 | share cut > 0.5 m at uphill edge | share fill > 0.5 m at downhill edge | pieces with fill at downhill edge | fill extent m p50 p90 | share of those not met within 60 m"
    );
    for (lo, hi) in bands {
        let list: Vec<&Cf> = cfs
            .iter()
            .filter(|c| c.slope >= lo && c.slope < hi)
            .collect();
        if list.is_empty() {
            continue;
        }
        let mut u: Vec<f64> = list.iter().map(|c| c.up).collect();
        let mut m: Vec<f64> = list.iter().map(|c| c.mid).collect();
        let mut d: Vec<f64> = list.iter().map(|c| c.down).collect();
        let mut ex: Vec<f64> = list.iter().filter_map(|c| c.extent).collect();
        let n = list.len() as f64;
        println!(
            "CF {:3.0}-{:<3.0}% | {:5} | {:6.1} | {:6.2} {:6.2} | {:6.2} {:6.2} | {:6.2} {:6.2} | {:.2} | {:.2} | {:5} | {:5.1} {:5.1} | {:.2}",
            lo * 100.0,
            hi.min(9.99) * 100.0,
            list.len(),
            list.iter().map(|c| c.len).sum::<f64>() / 1000.0,
            pct(&mut u, 0.5),
            pct(&mut u, 0.9),
            pct(&mut m, 0.5),
            pct(&mut m, 0.9),
            pct(&mut d, 0.5),
            pct(&mut d, 0.9),
            list.iter().filter(|c| c.up < -0.5).count() as f64 / n,
            list.iter().filter(|c| c.down > 0.5).count() as f64 / n,
            ex.len(),
            pct(&mut ex, 0.5),
            pct(&mut ex, 0.9),
            list.iter()
                .filter(|c| c.extent.is_some_and(|e| e >= 60.0))
                .count() as f64
                / ex.len().max(1) as f64
        );
        println!(
            "CFX {:3.0}-{:<3.0}% | share of pieces with fill at the downhill edge > 2 m {:.2}, > 5 m {:.2} | share with the whole width in cut (downhill edge not above the original by more than 0.25 m, uphill edge below it) {:.2} | share with the centre line above the original by more than 0.5 m {:.2}",
            lo * 100.0,
            hi.min(9.99) * 100.0,
            list.iter().filter(|c| c.down > 2.0).count() as f64 / n,
            list.iter().filter(|c| c.down > 5.0).count() as f64 / n,
            list.iter().filter(|c| c.down <= 0.25 && c.up < 0.0).count() as f64 / n,
            list.iter().filter(|c| c.mid > 0.5).count() as f64 / n
        );
    }

    // ---- the extremes, checked by hand
    println!("\n#### SECTION: places to look at");
    let inspect_radius: f64 = std::env::var("INSPECT_R")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    let inspect = |label: &str, x: f64, y: f64| {
        println!(
            "\nINSPECT {label} at ({x:.1},{y:.1}): base {:.1} m, finished {:.1} m, base slope {:.0} deg",
            base.sample(x, y).height,
            hm.sample(x, y).height,
            base.sample(x, y).slope_deg()
        );
        for v in violations
            .iter()
            .filter(|v| (v.at.0 - x).hypot(v.at.1 - y) <= 50.0)
        {
            println!(
                "  violation {} ({:.1},{:.1}) {}",
                v.rule.id(),
                v.at.0,
                v.at.1,
                v.message
            );
        }
        let mut close: Vec<(f64, usize)> = segs
            .iter()
            .enumerate()
            .map(|(i, g)| (seg_dist((x, y), (g.a.0, g.a.1), (g.b.0, g.b.1)).0, i))
            .filter(|(d, _)| *d <= inspect_radius)
            .collect();
        close.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut dir_here = (1.0, 0.0);
        for (n, (d, i)) in close.iter().take(24).enumerate() {
            let g = &segs[*i];
            let l = (g.b.0 - g.a.0).hypot(g.b.1 - g.a.1).max(0.5);
            if n == 0 {
                dir_here = ((g.b.0 - g.a.0) / l, (g.b.1 - g.a.1) / l);
            }
            let slot = strips[g.strip].slots.get(g.index).copied();
            let road = slot.map(|s| &net.roads[s]);
            let chk = slot.and_then(|s| by_slot.get(&s));
            println!(
                "  seg {i} d {d:5.1} strip {} stretch {} idx {} {:?} {:?} drv {} {} a({:.0},{:.0}) h {:.2} b({:.0},{:.0}) h {:.2} planned {:.0}% | terrain {:.2}/{:.2} base {:.2}/{:.2} | check grade {} fall {} miss {} {}",
                g.strip,
                road.map_or("?", |r| r.slot.stretch.as_str()),
                road.map_or(0, |r| r.slot.index),
                road.map(|r| r.kind),
                road.map(|r| r.family),
                road.is_some_and(|r| r.drivable),
                road.map_or("", |r| r.model.rsplit('\\').next().unwrap_or("")),
                g.a.0,
                g.a.1,
                g.a.2,
                g.b.0,
                g.b.1,
                g.b.2,
                (g.b.2 - g.a.2) / l * 100.0,
                hm.sample(g.a.0, g.a.1).height,
                hm.sample(g.b.0, g.b.1).height,
                base.sample(g.a.0, g.a.1).height,
                base.sample(g.b.0, g.b.1).height,
                chk.map_or("-".to_string(), |c| format!("{:.0}%", c.grade * 100.0)),
                chk.map_or("-".to_string(), |c| format!("{:.0}%", c.cross_fall * 100.0)),
                chk.map_or("-".to_string(), |c| format!("{:.2}", c.miss_m)),
                chk.map_or(String::new(), |c| format!("{:?}", c.state))
            );
        }
        // the limits the earthworks put on a vertex here: the highest "lower" and the lowest "upper" over all
        // road parts within 200 m (straight-line approximation of the chamfer distance, from the core's edge)
        {
            let nat = f64::from(base.sample(x, y).slope_deg()).to_radians().tan();
            let (fill, cut) = ((1.5 * nat).clamp(0.667, 1.0), (1.5 * nat).clamp(1.0, 2.0));
            let mut lows: Vec<(f64, usize, f64)> = Vec::new();
            let mut ups: Vec<(f64, usize, f64)> = Vec::new();
            for (i, g) in segs.iter().enumerate() {
                if g.bridge {
                    continue;
                }
                let (d, t) = seg_dist((x, y), (g.a.0, g.a.1), (g.b.0, g.b.1));
                let reach = strips[g.strip].half_width + 1.5 * cell;
                let dc = (d - reach).max(0.0);
                if dc > 200.0 {
                    continue;
                }
                let h = g.a.2 + t * (g.b.2 - g.a.2);
                lows.push((h - fill * dc, i, dc));
                ups.push((h + cut * dc, i, dc));
            }
            lows.sort_by(|a, b| b.0.total_cmp(&a.0));
            ups.sort_by(|a, b| a.0.total_cmp(&b.0));
            if let (Some(l), Some(u)) = (lows.first(), ups.first()) {
                println!(
                    "  limits here (fill {fill:.2}, cut {cut:.2} from the slope at the point, distances from the core edge): highest lower bound {:.1} from seg {} (strip {}, {:.0} m away); lowest upper bound {:.1} from seg {} (strip {}, {:.0} m away); lower - upper = {:.1} m",
                    l.0,
                    l.1,
                    segs[l.1].strip,
                    l.2,
                    u.0,
                    u.1,
                    segs[u.1].strip,
                    u.2,
                    l.0 - u.0
                );
            }
        }
        for (name, d) in [("along", dir_here), ("across", (dir_here.1, -dir_here.0))] {
            let pts: Vec<(f64, f64)> = (-8..=8)
                .map(|k| (x + d.0 * 5.0 * f64::from(k), y + d.1 * 5.0 * f64::from(k)))
                .collect();
            let b = pts
                .iter()
                .map(|q| format!("{:.0}", base.sample(q.0, q.1).height))
                .collect::<Vec<_>>()
                .join(" ");
            let f = pts
                .iter()
                .map(|q| format!("{:.0}", hm.sample(q.0, q.1).height))
                .collect::<Vec<_>>()
                .join(" ");
            println!("  section {name} the nearest segment, -40..+40 m every 5 m: base {b}");
            println!("  section {name}                                              finished {f}");
        }
    };
    if let Some(p) = pieces
        .iter()
        .filter(|p| p.state == DriveState::NotDrivable)
        .max_by(|a, b| a.grade.total_cmp(&b.grade))
    {
        println!(
            "steepest not-drivable piece: grade {:.0}% fall {:.0}% len {:.1}",
            p.grade * 100.0,
            p.cross_fall * 100.0,
            len(p)
        );
        inspect(
            "steepest not-drivable piece",
            (p.start.0 + p.end.0) / 2.0,
            (p.start.1 + p.end.1) / 2.0,
        );
    }
    if let Some(p) = pieces
        .iter()
        .filter(|p| p.state == DriveState::NotDrivable)
        .max_by(|a, b| a.cross_fall.total_cmp(&b.cross_fall))
    {
        println!(
            "largest cross fall of a not-drivable piece: fall {:.0}% grade {:.0}% len {:.1}",
            p.cross_fall * 100.0,
            p.grade * 100.0,
            len(p)
        );
        inspect(
            "largest cross fall",
            (p.start.0 + p.end.0) / 2.0,
            (p.start.1 + p.end.1) / 2.0,
        );
    }
    if let Some(e) = profiles
        .excesses
        .iter()
        .filter(|e| e.kind == naves_core::profile::GradeExcessKind::Grade)
        .max_by(|a, b| a.grade.abs().total_cmp(&b.grade.abs()))
    {
        println!(
            "largest road_grade: {:.0}% over limit {:.0}%",
            e.grade * 100.0,
            e.limit * 100.0
        );
        inspect("largest road_grade", e.at.0, e.at.1);
    }
    if let Some(g) = net.gaps.iter().max_by(|a, b| a.gap_m.total_cmp(&b.gap_m)) {
        println!(
            "largest road_gap: stretch {} gap {:.1} m heading error {:.1} deg at ({:.1},{:.1})",
            g.stretch, g.gap_m, g.heading_error_deg, g.at.0, g.at.1
        );
        if let Some(st) = graph.stretches.iter().find(|s| s.id == g.stretch) {
            let pl: f64 = st
                .points
                .windows(2)
                .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
                .sum();
            println!(
                "  stretch {:?} drivable {} bridge {} tunnel {} {} points, {:.1} m long, from ({:.1},{:.1}) to ({:.1},{:.1})",
                st.family,
                st.drivable,
                st.bridge,
                st.tunnel,
                st.points.len(),
                pl,
                st.points[0].0,
                st.points[0].1,
                st.points[st.points.len() - 1].0,
                st.points[st.points.len() - 1].1
            );
        }
        let placed: Vec<&naves_core::roadplace::PlacedRoad> = net
            .roads
            .iter()
            .filter(|r| r.slot.stretch == g.stretch)
            .collect();
        println!("  placed objects on it: {}", placed.len());
        for r in placed.iter().take(12) {
            println!(
                "  object {:?} {} from ({:.1},{:.1}) to ({:.1},{:.1})",
                r.kind,
                r.model.rsplit('\\').next().unwrap_or(""),
                r.start.x,
                r.start.y,
                r.end.x,
                r.end.y
            );
        }
        inspect("largest road_gap", g.at.0, g.at.1);
    }
    if let Some(c) = conformed
        .conflicts
        .iter()
        .max_by(|a, b| a.height_difference_m.total_cmp(&b.height_difference_m))
    {
        println!("largest road_conflict: {:.1} m", c.height_difference_m);
        inspect("largest road_conflict", c.at.0, c.at.1);
    }
    if let Ok(list) = std::env::var("INSPECT") {
        for item in list.split(';').filter(|s| !s.is_empty()) {
            let mut it = item
                .split(',')
                .map(|v| v.trim().parse::<f64>().expect("number"));
            if let (Some(x), Some(y)) = (it.next(), it.next()) {
                inspect("requested", x, y);
            }
        }
    }

    // ---- strips on request: STRIP=1027,983 prints every joint
    if let Ok(list) = std::env::var("STRIP") {
        for item in list.split(',').filter(|s| !s.is_empty()) {
            let si: usize = item.trim().parse().expect("strip index");
            let st = &strips[si];
            println!(
                "\nSTRIP {si}: {} joints, half width {:.1}, rank {}, first slot {:?}, last slot {:?}",
                st.joints.len(),
                st.half_width,
                st.rank,
                st.slots.first().map(|s| &net.roads[*s].slot.stretch),
                st.slots.last().map(|s| &net.roads[*s].slot.stretch)
            );
            println!(
                "  joint along | x y | planned | base | finished | planned-base | next grade % | flags"
            );
            let mut along = 0.0;
            for (i, j) in st.joints.iter().enumerate() {
                let next = st.joints.get(i + 1);
                let g = next.map(|n| (n.2 - j.2) / (n.0 - j.0).hypot(n.1 - j.1).max(0.5) * 100.0);
                println!(
                    "  {i:3} {along:7.1} | {:7.1} {:7.1} | {:7.2} | {:7.2} | {:7.2} | {:7.2} | {} | {}{}",
                    j.0,
                    j.1,
                    j.2,
                    base.sample(j.0, j.1).height,
                    hm.sample(j.0, j.1).height,
                    j.2 - f64::from(base.sample(j.0, j.1).height),
                    g.map_or("-".to_string(), |g| format!("{g:.0}")),
                    if st.bridge.get(i).copied().unwrap_or(false) {
                        "bridge "
                    } else {
                        ""
                    },
                    if st.junction.get(i).copied().unwrap_or(false) {
                        "junction"
                    } else {
                        ""
                    }
                );
                if let Some(n) = next {
                    along += (n.0 - j.0).hypot(n.1 - j.1);
                }
            }
        }
    }

    // spans between two held joints that `ramp_targets` turns into a straight ramp at the first (aim) grade
    // limit: by the lowest limit of the span (the rule before 2026-11-13's fix) and by the mean by length
    {
        let (mut old_n, mut old_len, mut new_n, mut new_len) = (0, 0.0, 0, 0.0);
        let mut spans = 0;
        for st in strips {
            let holds = chain_holds(&net, &st.slots, f64::from(base.cell_m), &junctions);
            let held: Vec<usize> = holds
                .iter()
                .enumerate()
                .filter(|(_, (_, h))| h.is_some())
                .map(|(i, _)| i)
                .collect();
            for pair in held.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if b <= a + 1 {
                    continue;
                }
                spans += 1;
                let aim = grade_aim(net.roads[st.slots[a]].family);
                let (mut run, mut weighted, mut min_factor) = (0.0, 0.0, 1.0_f64);
                for k in a..b {
                    let (p, q) = (st.joints[k], st.joints[k + 1]);
                    let length = (q.0 - p.0).hypot(q.1 - p.1).max(0.5);
                    let road = &net.roads[st.slots[k]];
                    let turn = (road.end.heading - road.start.heading + std::f64::consts::PI)
                        .rem_euclid(std::f64::consts::TAU)
                        - std::f64::consts::PI;
                    let mut factor = 1.0;
                    if turn.abs() > 0.001 {
                        let chord = (road.end.x - road.start.x).hypot(road.end.y - road.start.y);
                        let radius = chord / (2.0 * (turn.abs() / 2.0).sin());
                        factor = ((radius - road.width / 2.0) / radius).max(0.5);
                    }
                    min_factor = min_factor.min(factor);
                    run += length;
                    weighted += factor * length;
                }
                let (ha, hb) = (
                    holds[a].1.map_or(0.0, |h| h.height),
                    holds[b].1.map_or(0.0, |h| h.height),
                );
                let required = (hb - ha).abs() / run;
                if required >= 0.8 * aim * min_factor {
                    old_n += 1;
                    old_len += run;
                }
                if required >= 0.8 * aim * weighted / run {
                    new_n += 1;
                    new_len += run;
                }
            }
        }
        println!(
            "RAMPSPANS spans {spans}; straight at the aim by the lowest limit {old_n} ({:.1} km), by the mean limit {new_n} ({:.1} km)",
            old_len / 1000.0,
            new_len / 1000.0
        );
    }

    // ---- DIAG=x,y,r: everything about the roads, junctions and ground near a place
    if let Ok(spec) = std::env::var("DIAG") {
        let v: Vec<f64> = spec
            .split(',')
            .map(|v| v.trim().parse().expect("number"))
            .collect();
        let (cx, cy, r) = (v[0], v[1], v.get(2).copied().unwrap_or(60.0));
        let h_of = |m: &Heightmap, p: (f64, f64)| f64::from(m.sample(p.0, p.1).height);
        println!("\nDIAG at ({cx},{cy}) radius {r}");
        let near = |poly: &[(f64, f64)], rr: f64| {
            poly.windows(2)
                .any(|w| seg_dist((cx, cy), w[0], w[1]).0 <= rr)
        };
        for f in features.iter().filter(|f| near(&f.points, r)) {
            let tags = ways.ways.iter().find(|w| w.id == f.way_id).map(|w| &w.tags);
            println!(
                "DIAG feature {} way {} {:?} tags {tags:?}",
                f.id, f.way_id, f.kind
            );
            let first = f.points.first().copied().unwrap_or((0.0, 0.0));
            let last = f.points.last().copied().unwrap_or((0.0, 0.0));
            println!(
                "  {} points, nodes {:?}..{:?}, ({:.0},{:.0}) to ({:.0},{:.0}), base {:.1} to {:.1}",
                f.points.len(),
                f.nodes.first(),
                f.nodes.last(),
                first.0,
                first.1,
                last.0,
                last.1,
                h_of(&base, first),
                h_of(&base, last)
            );
            for (i, pt) in f.points.iter().enumerate() {
                println!(
                    "    node {} ({:.0},{:.0}) base {:.1}",
                    f.nodes[i],
                    pt.0,
                    pt.1,
                    h_of(&base, *pt)
                );
            }
        }
        for (si, st) in strips.iter().enumerate() {
            if !st.joints.iter().any(|j| (j.0 - cx).hypot(j.1 - cy) <= r) {
                continue;
            }
            let stretch = st.slots.first().map(|s| net.roads[*s].slot.stretch.clone());
            println!(
                "DIAG strip {si} stretch {stretch:?} family {:?} rank {} joints {}",
                st.slots.first().map(|s| net.roads[*s].family),
                st.rank,
                st.joints.len()
            );
            let holds = chain_holds(&net, &st.slots, f64::from(base.cell_m), &junctions);
            for (i, (at, hold)) in holds.iter().enumerate() {
                if let Some(h) = hold {
                    println!(
                        "  HOLD joint {i} ({:.0},{:.0}) {:?} junction {} height {:.2}",
                        at.0, at.1, h.rule, h.junction, h.height
                    );
                }
            }
            // the ways under the joints of the chain
            let mut chain_ways: BTreeMap<i64, usize> = BTreeMap::new();
            for j in &st.joints {
                for f in &features {
                    if f.points
                        .windows(2)
                        .any(|w| seg_dist((j.0, j.1), w[0], w[1]).0 < 4.0)
                    {
                        *chain_ways.entry(f.way_id).or_default() += 1;
                    }
                }
            }
            for (way, n) in &chain_ways {
                let w = ways.ways.iter().find(|w| w.id == *way);
                let kind = features.iter().find(|f| f.way_id == *way).map(|f| &f.kind);
                println!(
                    "  CHAINWAY {way} ({n} joints within 4 m) {kind:?} tags {:?}",
                    w.map(|w| &w.tags)
                );
            }
            // the spans between two held joints: what the solver sees (profile.rs `ramp_targets`)
            let held: Vec<usize> = holds
                .iter()
                .enumerate()
                .filter(|(_, (_, h))| h.is_some())
                .map(|(i, _)| i)
                .collect();
            for pair in held.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let (ha, hb) = (
                    holds[a].1.map_or(0.0, |h| h.height),
                    holds[b].1.map_or(0.0, |h| h.height),
                );
                let mut run = 0.0;
                let mut min_factor = 1.0_f64;
                let mut tightest = (0, f64::MAX);
                for k in a..b {
                    let (p, q) = (st.joints[k], st.joints[k + 1]);
                    run += (q.0 - p.0).hypot(q.1 - p.1).max(0.5);
                    let road = &net.roads[st.slots[k]];
                    let turn = (road.end.heading - road.start.heading + std::f64::consts::PI)
                        .rem_euclid(std::f64::consts::TAU)
                        - std::f64::consts::PI;
                    if turn.abs() > 0.001 {
                        let chord = (road.end.x - road.start.x).hypot(road.end.y - road.start.y);
                        let radius = chord / (2.0 * (turn.abs() / 2.0).sin());
                        let factor = ((radius - road.width / 2.0) / radius).max(0.5);
                        if factor < min_factor {
                            min_factor = factor;
                        }
                        if radius < tightest.1 {
                            tightest = (k, radius);
                        }
                    }
                }
                let required = (hb - ha).abs() / run.max(0.5);
                let aim = grade_aim(net.roads[st.slots[a]].family);
                let ceil = naves_core::limits::grade_ceiling(net.roads[st.slots[a]].family);
                println!(
                    "  SPAN joints {a}-{b}: {ha:.2} to {hb:.2}, run {run:.0} m, required grade {:.3}; smallest bend radius {:.0} m at piece {} (factor {min_factor:.2})",
                    required, tightest.1, tightest.0
                );
                for base_grade in [aim, f64::midpoint(aim, ceil), ceil] {
                    let limit = base_grade * min_factor;
                    println!(
                        "    base grade {base_grade:.3}: span limit {limit:.3}, required/limit {:.2}, tight (>= 0.8) {}, infeasible {}",
                        required / limit,
                        required >= 0.8 * limit,
                        required > limit
                    );
                }
            }
            println!("  i | x y | planned | base | finished | planned-base | flags | side slope");
            for (i, j) in st.joints.iter().enumerate() {
                println!(
                    "  {i:3} | {:7.1} {:7.1} | {:7.2} | {:7.2} | {:7.2} | {:7.2} | {}{} | {:.2}",
                    j.0,
                    j.1,
                    j.2,
                    h_of(&base, (j.0, j.1)),
                    h_of(hm, (j.0, j.1)),
                    j.2 - h_of(&base, (j.0, j.1)),
                    if st.bridge.get(i).copied().unwrap_or(false) {
                        "bridge "
                    } else {
                        ""
                    },
                    if st.junction.get(i).copied().unwrap_or(false) {
                        "junction"
                    } else {
                        ""
                    },
                    st.side_slope.get(i).copied().unwrap_or(0.0)
                );
            }
        }
        for (ji, j) in junctions.points.iter().enumerate() {
            if (j.at.0 - cx).hypot(j.at.1 - cy) <= r + 400.0 {
                println!(
                    "DIAG junction {ji} at ({:.0},{:.0}) {:?} ground {:.2} height {:.2} (moved {:.2}) rank {} stretches {:?}, {:.0} m away",
                    j.at.0,
                    j.at.1,
                    j.kind,
                    j.ground,
                    j.height,
                    j.height - j.ground,
                    j.rank,
                    j.stretches,
                    (j.at.0 - cx).hypot(j.at.1 - cy)
                );
            }
        }
        for (li, l) in junctions.links.iter().enumerate() {
            let (a, b) = (&junctions.points[l.a], &junctions.points[l.b]);
            if (a.at.0 - cx).hypot(a.at.1 - cy) <= r + 400.0
                || (b.at.0 - cx).hypot(b.at.1 - cy) <= r + 400.0
            {
                println!(
                    "DIAG link {li}: junction {} - {} length {:.0} ceiling {:.2} height difference {:.2}",
                    l.a,
                    l.b,
                    l.length,
                    l.ceiling,
                    (a.height - b.height).abs()
                );
            }
        }
        println!(
            "DIAG ground along x (y = {cy}) and along y (x = {cx}), base every 10 m, -100..100:"
        );
        for (name, dir) in [("across x", (1.0, 0.0)), ("across y", (0.0, 1.0))] {
            let row: Vec<String> = (-10..=10)
                .map(|k| {
                    let t = f64::from(k) * 10.0;
                    format!("{:.0}", h_of(&base, (cx + dir.0 * t, cy + dir.1 * t)))
                })
                .collect();
            println!("  {name}: {}", row.join(" "));
        }
        let (mut lo, mut hi, mut step, mut nan) = (f64::MAX, f64::MIN, 0.0_f64, 0);
        let cell = f64::from(base.cell_m);
        let mut t = -50.0;
        while t <= 50.0 {
            let mut u = -50.0;
            while u <= 50.0 {
                let h = h_of(&base, (cx + t, cy + u));
                if h.is_nan() {
                    nan += 1;
                } else {
                    lo = lo.min(h);
                    hi = hi.max(h);
                }
                step = step.max((h - h_of(&base, (cx + t + cell, cy + u))).abs());
                step = step.max((h - h_of(&base, (cx + t, cy + u + cell))).abs());
                u += cell;
            }
            t += cell;
        }
        println!(
            "DIAG base within 50 m: min {lo:.1} max {hi:.1}, largest step between neighbouring cells {step:.1} m, NaN {nan}, cell {cell} m"
        );
        // the DEM itself: raw samples every 10 m through the centre
        for (name, dir) in [("DEM along x", (1.0, 0.0)), ("DEM along y", (0.0, 1.0))] {
            let row: Vec<String> = (-10..=10)
                .map(|k| {
                    let t = f64::from(k) * 10.0;
                    frame
                        .to_geo(cx + dir.0 * t, cy + dir.1 * t)
                        .ok()
                        .and_then(|(lat, lon)| dem.height(lat, lon))
                        .map_or("none".to_string(), |h| format!("{h:.0}"))
                })
                .collect();
            println!("  {name}: {}", row.join(" "));
        }
    }

    // ---- done: the criteria of the road heights plan (2026-11-13)
    println!("\n#### done");
    println!(
        "junctions {} (pieces {}, joins {}, ends on roads {}), links {}, unresolved {}",
        junctions.points.len(),
        junctions
            .points
            .iter()
            .filter(|j| j.kind == naves_core::junctions::JunctionKind::Piece)
            .count(),
        junctions
            .points
            .iter()
            .filter(|j| j.kind == naves_core::junctions::JunctionKind::Join)
            .count(),
        junctions
            .points
            .iter()
            .filter(|j| j.kind == naves_core::junctions::JunctionKind::EndOnRoad)
            .count(),
        junctions.links.len(),
        junctions.unresolved.len()
    );
    // (1) planned grade above 100 %, (2) above the hard limit without a violation near it
    let covered = |x: f64, y: f64| {
        violations.iter().any(|v| {
            matches!(v.rule, Rule::RoadGrade | Rule::RoadNotDrivable)
                && (v.at.0 - x).hypot(v.at.1 - y) <= 30.0
        })
    };
    let (mut over_100, mut over_hard, mut over_hard_unreported) = (0, 0, 0);
    let mut worst_planned = (0.0_f64, (0.0, 0.0), String::new());
    let mut causes: BTreeMap<String, usize> = BTreeMap::new();
    let net_ref = &net;
    for (strip_index, strip) in profiles.strips.iter().enumerate() {
        for (k, w) in strip.joints.windows(2).enumerate() {
            let run = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
            if run < 0.5 || strip.bridge.get(k).copied().unwrap_or(false) {
                continue;
            }
            let grade = ((w[1].2 - w[0].2) / run).abs();
            let road = strip.slots.get(k).and_then(|i| net.roads.get(*i));
            if grade > worst_planned.0 {
                worst_planned = (
                    grade,
                    (w[0].0, w[0].1),
                    road.map(|r| r.slot.stretch.clone()).unwrap_or_default(),
                );
            }
            if grade > 1.0 {
                over_100 += 1;
                println!(
                    "  planned grade {:.0} % at ({:.1},{:.1}) stretch {} strip {} piece {}",
                    grade * 100.0,
                    w[0].0,
                    w[0].1,
                    road.map_or("?", |r| r.slot.stretch.as_str()),
                    strip_index,
                    k
                );
                let cause =
                    explain_piece(net_ref, strip, k, &junctions, f64::from(base.cell_m), &base);
                *causes.entry(cause).or_default() += 1;
            }
            if let Some(road) = road
                && grade > drive_grade(road.family)
            {
                over_hard += 1;
                if !covered(w[0].0, w[0].1) && !covered(w[1].0, w[1].1) {
                    over_hard_unreported += 1;
                    let near_excess: Vec<String> = profiles
                        .excesses
                        .iter()
                        .filter(|e| (e.at.0 - w[0].0).hypot(e.at.1 - w[0].1) <= 60.0)
                        .map(|e| {
                            format!(
                                "{:?} {:.0} % > {:.0} % at ({:.1},{:.1})",
                                e.kind,
                                e.grade * 100.0,
                                e.limit * 100.0,
                                e.at.0,
                                e.at.1
                            )
                        })
                        .collect();
                    let first_family = strip
                        .slots
                        .first()
                        .and_then(|i| net.roads.get(*i))
                        .map(|r| r.family);
                    let kinds: Vec<String> = (k.saturating_sub(2)..(k + 3).min(strip.slots.len()))
                        .filter_map(|i| {
                            let r = net.roads.get(*strip.slots.get(i)?)?;
                            let j0 = strip.joints.get(i)?;
                            let j1 = strip.joints.get(i + 1)?;
                            Some(format!(
                                "[{i}: {:?} len {:.1} run {:.1} dh {:.2} start-gap {:.1}]",
                                r.kind,
                                (r.end.x - r.start.x).hypot(r.end.y - r.start.y),
                                (j1.0 - j0.0).hypot(j1.1 - j0.1),
                                j1.2 - j0.2,
                                (r.start.x - j0.0).hypot(r.start.y - j0.1)
                            ))
                        })
                        .collect();
                    println!("      pieces {kinds:?}");
                    println!(
                        "  UNREPORTED grade {:.0} % (hard {:.0} %) at ({:.1},{:.1}) strip {strip_index} piece {k} kind {:?} family {:?} chain family {:?}; excesses within 60 m: {near_excess:?}",
                        grade * 100.0,
                        drive_grade(road.family) * 100.0,
                        w[0].0,
                        w[0].1,
                        road.kind,
                        road.family,
                        first_family
                    );
                }
            }
        }
    }
    println!("DONE 1 pieces with a planned grade above 100 %: {over_100}");
    for (cause, n) in &causes {
        println!("DONE 1 cause {n:4} {cause}");
    }
    if let Ok(v) = std::env::var("DBG_STRIP") {
        let si: usize = v.parse().unwrap();
        let st = &profiles.strips[si];
        let holds = chain_holds(&net, &st.slots, f64::from(base.cell_m), &junctions);
        for (i, ((x, y), h)) in holds.iter().enumerate() {
            println!(
                "STRIP {si} joint {i} ({x:.1},{y:.1}) h {:.2} ground {:.2} {}",
                st.joints[i].2,
                base.sample(*x, *y).height,
                h.map_or(String::from("free"), |h| format!(
                    "{:?} j{} {:.2}",
                    h.rule, h.junction, h.height
                ))
            );
        }
    }
    held_pairs(&net, &profiles, &junctions, f64::from(base.cell_m));
    hold_report(&net, &profiles, &junctions, f64::from(base.cell_m));
    bend_report(&net, &profiles, &junctions, &base);
    println!(
        "DONE 2 pieces above the hard limit (drive_grade): {over_hard}, of them without a road_grade or road_not_drivable violation within 30 m: {over_hard_unreported}"
    );
    println!(
        "steepest planned piece {:.0} % at ({:.1},{:.1}) stretch {}",
        worst_planned.0 * 100.0,
        worst_planned.1.0,
        worst_planned.1.1,
        worst_planned.2
    );
    // (3) the junction's own height against its ground median, and what the profile has there
    let mut from_ground: Vec<f64> = junctions
        .points
        .iter()
        .map(|j| (j.height - j.ground).abs())
        .collect();
    println!(
        "DONE 3 junction height from its ground median: p50 {:.2} p90 {:.2} max {:.2} m",
        pct(&mut from_ground, 0.5),
        pct(&mut from_ground, 0.9),
        pct(&mut from_ground, 1.0)
    );
    let mut planned: Vec<f64> = Vec::new();
    for j in &junctions.points {
        let nearest = profiles
            .strips
            .iter()
            .flat_map(|s| s.joints.iter())
            .map(|q| ((q.0 - j.at.0).hypot(q.1 - j.at.1), q.2))
            .filter(|(d, _)| *d <= j.patch_m + HOLD_M)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, height)) = nearest {
            planned.push((height - j.ground).abs());
            if (height - j.ground).abs() > 3.0 {
                println!(
                    "  junction at ({:.1},{:.1}) {:?}: planned {height:.1}, ground median {:.1}, height {:.1}",
                    j.at.0, j.at.1, j.kind, j.ground, j.height
                );
            }
        }
    }
    println!(
        "planned junction height from the ground median (was p50 1.64, p90 6.80): p50 {:.2} p90 {:.2} max {:.2} m ({} junctions with a joint)",
        pct(&mut planned, 0.5),
        pct(&mut planned, 0.9),
        pct(&mut planned, 1.0),
        planned.len()
    );
    let worst_excess = profiles
        .excesses
        .iter()
        .filter(|e| e.kind == naves_core::profile::GradeExcessKind::Grade)
        .max_by(|a, b| a.grade.abs().total_cmp(&b.grade.abs()));
    println!(
        "largest road_grade excess {:.0} %{}",
        worst_excess.map_or(0.0, |e| e.grade.abs() * 100.0),
        worst_excess.map_or(String::new(), |e| format!(
            " at ({:.1},{:.1})",
            e.at.0, e.at.1
        ))
    );

    // ---- cost
    let hwm = std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("VmHWM"))
        .unwrap_or("")
        .to_string();
    println!(
        "\nCOST size {} wall {:.1} s (to the end of the analysis), {hwm}",
        base.size,
        t0.elapsed().as_secs_f64()
    );
}

/// The held joints of a strip within 50 m along the chain of piece `k`, printed, and the cause of a steep piece:
/// the rules and junctions of the held joints on both sides and what `find_junctions` knows of the pair.
fn explain_piece(
    net: &naves_core::roadplace::PlacedNetwork,
    strip: &RoadStrip,
    k: usize,
    junctions: &naves_core::junctions::Junctions,
    cell: f64,
    ground: &Heightmap,
) -> String {
    let holds = chain_holds(net, &strip.slots, cell, junctions);
    let mut along = vec![0.0];
    for w in strip.joints.windows(2) {
        let last = along.last().copied().unwrap_or(0.0);
        along.push(last + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
    }
    let here = along.get(k).copied().unwrap_or(0.0);
    for (i, ((x, y), hold)) in holds.iter().enumerate() {
        let d = along.get(i).copied().unwrap_or(0.0) - here;
        if d.abs() <= 120.0 && hold.is_none() {
            println!(
                "      joint {i} at {d:+.1} m ({x:.1},{y:.1}) free, joint height {:.2} ground {:.2}",
                strip.joints.get(i).map_or(0.0, |q| q.2),
                ground.sample(*x, *y).height
            );
        }
        if d.abs() <= 120.0
            && let Some(h) = hold
        {
            let j = &junctions.points[h.junction];
            println!(
                "      joint {i} at {d:+.1} m ({x:.1},{y:.1}) height {:.2} (joint height {:.2}): {:?} junction {} {:?} at ({:.1},{:.1}) ground {:.2}",
                h.height,
                strip.joints.get(i).map_or(0.0, |q| q.2),
                h.rule,
                h.junction,
                j.kind,
                j.at.0,
                j.at.1,
                j.ground
            );
        }
    }
    let before = (0..=k)
        .rev()
        .find_map(|i| holds.get(i).and_then(|(_, h)| h.map(|h| (i, h))));
    let after =
        (k + 1..holds.len()).find_map(|i| holds.get(i).and_then(|(_, h)| h.map(|h| (i, h))));
    let deck = strip.bridge.iter().any(|b| *b);
    let state = match (before, after) {
        (Some((i, a)), Some((m, b))) => {
            let gap = along.get(m).copied().unwrap_or(0.0) - along.get(i).copied().unwrap_or(0.0);
            let rise = (a.height - b.height).abs();
            if a.junction == b.junction {
                format!("{:?}/{:?} same junction", a.rule, b.rule)
            } else {
                let link = junctions.links.iter().position(|l| {
                    (l.a, l.b) == (a.junction.min(b.junction), a.junction.max(b.junction))
                });
                match link {
                    None => format!("{:?}/{:?} different junctions, no link", a.rule, b.rule),
                    Some(li) => {
                        let l = &junctions.links[li];
                        format!(
                            "{:?}/{:?} link {} (unresolved {}): rise {rise:.1} m over {gap:.0} m held-to-held, link length {:.0} m ceiling {:.2}",
                            a.rule,
                            b.rule,
                            li,
                            junctions.unresolved.contains(&li),
                            l.length,
                            l.ceiling
                        )
                    }
                }
            }
        }
        (None, None) => "no held joint on the chain".to_string(),
        _ => "held joint on one side only".to_string(),
    };
    let _ = deck;
    // the key groups by what matters, not by numbers
    state
        .split(" link ")
        .next()
        .unwrap_or("")
        .split(": rise")
        .next()
        .unwrap_or("")
        .to_string()
        + if state.contains(" link ") {
            if state.contains("(unresolved true)") {
                " [link, unresolved]"
            } else {
                " [link]"
            }
        } else {
            ""
        }
}

/// The invariant of the heights: for any two consecutive held joints of a chain that take their heights from
/// different junctions, a link between those junctions exists, and unless it is unresolved the grade between the
/// two heights, over the distance along the chain, is within the link's ceiling. Prints the pairs that break it.
fn held_pairs(
    net: &naves_core::roadplace::PlacedNetwork,
    profiles: &naves_core::profile::Profiles,
    junctions: &naves_core::junctions::Junctions,
    cell: f64,
) {
    let (mut pairs, mut no_link, mut over) = (0, 0, 0);
    for (si, strip) in profiles.strips.iter().enumerate() {
        let holds = chain_holds(net, &strip.slots, cell, junctions);
        let mut along = vec![0.0];
        for w in strip.joints.windows(2) {
            let last = along.last().copied().unwrap_or(0.0);
            along.push(last + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
        }
        let mut previous: Option<(usize, naves_core::profile::Hold)> = None;
        for (i, (_, hold)) in holds.iter().enumerate() {
            let Some(h) = hold else { continue };
            if let Some((pi, p)) = previous
                && p.junction != h.junction
            {
                pairs += 1;
                let key = (p.junction.min(h.junction), p.junction.max(h.junction));
                let distance0 = (along[i] - along[pi]).max(1.0);
                let link = junctions
                    .links
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| (l.a, l.b) == key)
                    .min_by(|x, y| {
                        (x.1.length - distance0)
                            .abs()
                            .total_cmp(&(y.1.length - distance0).abs())
                    });
                let distance = (along[i] - along[pi]).max(0.5);
                match link {
                    None => {
                        no_link += 1;
                        println!(
                            "  PAIR no link: strip {si} joints {pi}-{i} junctions {} and {} ({:?}/{:?}) {distance:.0} m apart, rise {:.1} m",
                            p.junction,
                            h.junction,
                            p.rule,
                            h.rule,
                            (p.height - h.height).abs()
                        );
                    }
                    Some((li, l)) => {
                        let grade = (p.height - h.height).abs() / distance;
                        if (p.height - h.height).abs() > l.ceiling * distance + 0.011
                            && !junctions.unresolved.contains(&li)
                        {
                            over += 1;
                            println!(
                                "  PAIR over: strip {si} joints {pi}-{i} junctions {} and {} ({:?}/{:?}) grade {:.0} % over {distance:.0} m, ceiling {:.0} % (link length {:.0} m)",
                                p.junction,
                                h.junction,
                                p.rule,
                                h.rule,
                                grade * 100.0,
                                l.ceiling * 100.0,
                                l.length
                            );
                        }
                    }
                }
            }
            previous = Some((i, *h));
        }
    }
    println!(
        "DONE 1 invariant: consecutive held joints of different junctions {pairs}, without a link {no_link}, over the ceiling of a resolved link {over}"
    );
}

/// How much road the junctions hold level: the length of the runs of joints held to one junction that are
/// longer than one joint, and how many holds the check that a chain belongs to the junction removed.
fn hold_report(
    net: &naves_core::roadplace::PlacedNetwork,
    profiles: &naves_core::profile::Profiles,
    junctions: &naves_core::junctions::Junctions,
    cell: f64,
) {
    let (mut level_m, mut runs, mut held, mut unchecked) = (0.0, 0, 0, 0);
    for strip in &profiles.strips {
        let holds = chain_holds(net, &strip.slots, cell, junctions);
        held += holds.iter().filter(|(_, h)| h.is_some()).count();
        unchecked += chain_holds_unchecked(net, &strip.slots, cell, junctions)
            .iter()
            .filter(|(_, h)| h.is_some())
            .count();
        let mut start: Option<(usize, usize)> = None;
        for i in 0..=holds.len() {
            let here = holds.get(i).and_then(|(_, h)| h.map(|h| h.junction));
            match (start, here) {
                (Some((_, j)), Some(h)) if j == h => {}
                (Some((first, _)), _) => {
                    if i - 1 > first {
                        runs += 1;
                        level_m += (first..i - 1)
                            .map(|k| {
                                let (a, b) = (holds[k].0, holds[k + 1].0);
                                (b.0 - a.0).hypot(b.1 - a.1)
                            })
                            .sum::<f64>();
                    }
                    start = here.map(|h| (i, h));
                }
                (None, Some(h)) => start = Some((i, h)),
                (None, None) => {}
            }
        }
    }
    println!(
        "DONE held: {held} joints held to junctions; {runs} runs of more than one joint, {:.0} m of road held level; the check that a chain belongs to the junction removed {} holds (without it {unchecked})",
        level_m,
        unchecked - held
    );
}

/// Part 3 of the junction heights measurement: where the vertical-bend findings lie against the held joints, and
/// how far each junction height along a through chain would have to move to keep the minimum vertical radius.
///
/// The estimate: a run of joints held to one junction, on a chain that has a held run of another junction before
/// and after it, is at station `s` (the mean of its joints') at height `h`; its neighbours are at `s1`, `h1` and
/// `s2`, `h2`. The straight line through the neighbours has the height `h*` at `s`; the change of grade at `s`
/// may be at most `(d1 + d2) / 2 / R` per metre of run (`R` the minimum vertical radius of the road's family,
/// the same measure as the finding), which allows `h` to differ from `h*` by `(d1 + d2) / 2 / R / (1/d1 + 1/d2)`.
/// The move needed is the distance beyond that. The neighbours are not moved, so it is an upper bound on what
/// moving both would take.
fn bend_report(
    net: &naves_core::roadplace::PlacedNetwork,
    profiles: &naves_core::profile::Profiles,
    junctions: &naves_core::junctions::Junctions,
    base: &Heightmap,
) {
    let cell = f64::from(base.cell_m);
    let mut all_holds = Vec::new();
    for strip in &profiles.strips {
        all_holds.push(chain_holds(net, &strip.slots, cell, junctions));
    }
    // the bends by where they lie
    let (mut at_held, mut near_held, mut elsewhere, mut unmatched) = (0, 0, 0, 0);
    let mut bends = 0;
    for e in &profiles.excesses {
        if e.kind != naves_core::profile::GradeExcessKind::VerticalBend {
            continue;
        }
        bends += 1;
        let found = profiles
            .strips
            .iter()
            .zip(&all_holds)
            .find_map(|(strip, holds)| {
                strip
                    .joints
                    .iter()
                    .position(|q| (q.0 - e.at.0).abs() < 1e-6 && (q.1 - e.at.1).abs() < 1e-6)
                    .map(|i| (i, holds))
            });
        match found {
            None => unmatched += 1,
            Some((i, holds)) => {
                let is_held = |j: usize| holds.get(j).is_some_and(|(_, h)| h.is_some());
                if is_held(i) {
                    at_held += 1;
                } else if is_held(i + 1) || i.checked_sub(1).is_some_and(is_held) {
                    near_held += 1;
                } else {
                    elsewhere += 1;
                }
            }
        }
    }
    println!(
        "DONE bends: {bends} vertical-bend findings; at a held joint {at_held}, next to one {near_held}, elsewhere {elsewhere}, position not matched to a joint (bridge deck) {unmatched}; grade findings {}",
        profiles.excesses.len() - bends
    );
    // the moves
    let mut by_family: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for (strip, holds) in profiles.strips.iter().zip(&all_holds) {
        let Some(family) = strip
            .slots
            .first()
            .and_then(|i| net.roads.get(*i))
            .map(|r| r.family)
        else {
            continue;
        };
        let mut along = vec![0.0];
        for w in strip.joints.windows(2) {
            let last = along.last().copied().unwrap_or(0.0);
            along.push(last + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
        }
        // runs of joints held to one junction: (mean station, height)
        let mut runs: Vec<(usize, f64, f64)> = Vec::new();
        let mut sum = (0.0, 0usize);
        let mut current: Option<(usize, f64)> = None;
        for (i, (_, hold)) in holds.iter().enumerate() {
            match (hold, current) {
                (Some(h), Some((j, _))) if h.junction == j => {
                    sum.0 += along[i];
                    sum.1 += 1;
                }
                _ => {
                    if let Some((j, height)) = current.take() {
                        runs.push((j, sum.0 / sum.1 as f64, height));
                    }
                    if let Some(h) = hold {
                        current = Some((h.junction, h.height));
                        sum = (along[i], 1);
                    }
                }
            }
        }
        if let Some((j, height)) = current {
            runs.push((j, sum.0 / sum.1 as f64, height));
        }
        let radius = min_vertical_radius(family);
        for w in runs.windows(3) {
            let [(_, s1, h1), (_, s, h), (_, s2, h2)] = w else {
                continue;
            };
            let (d1, d2) = ((s - s1).max(0.5), (s2 - s).max(0.5));
            let straight = (h1 / d1 + h2 / d2) / (1.0 / d1 + 1.0 / d2);
            let allowed = (d1 + d2) / 2.0 / radius / (1.0 / d1 + 1.0 / d2);
            let need = ((h - straight).abs() - allowed).max(0.0);
            by_family
                .entry(format!("{family:?}"))
                .or_default()
                .push(need);
        }
    }
    let line = |name: &str, v: &mut Vec<f64>| {
        let n = v.len();
        let over = |t: f64| v.iter().filter(|x| **x > t).count();
        let (o5, o1, o2) = (over(0.5), over(1.0), over(2.0));
        println!(
            "DONE bends move needed {name:6} n {n:5}: p50 {:.2} p90 {:.2} max {:.2} m; above 0.5 m {o5}, above 1 m {o1}, above 2 m {o2}",
            pct(v, 0.5),
            pct(v, 0.9),
            pct(v, 1.0)
        );
    };
    let mut everything: Vec<f64> = by_family.values().flatten().copied().collect();
    line("all", &mut everything);
    for (family, v) in &mut by_family {
        line(family, v);
    }
}

// ======================================================================================================
// #### Shift study: how steep a ceiling lets a track lie on the ground, and what a sideways shift buys.
//
// Throwaway measurement of 2026-11-12. It works on the centre lines of the strips (the chains as placed),
// sampled every 5 m, against the original heightmap. It needs only the strips, so they can be dumped once
// (`SHIFT_DUMP=file`, after the 3 minute placing) and studied again from the dump (`SHIFT_LOAD=file`, which
// skips the placing and takes seconds). `SHIFT_ONLY=1` with `SHIFT_DUMP` studies and stops before the long
// sections. Output sections start with `####`.
// ======================================================================================================

use naves_core::catalog::Family;
use std::collections::HashMap;
use std::fmt::Write as _;

const SAMPLE_STEP_M: f64 = 5.0;
const OFFSET_STEP_M: f64 = 1.25;

#[derive(Clone)]
struct DStrip {
    fam: Family,
    drivable: bool,
    ids: Vec<String>,
    half_width: f64,
    joints: Vec<(f64, f64, f64)>,
    bridge: Vec<bool>,
    junction: Vec<bool>,
}

fn parse_family(s: &str) -> Family {
    match s {
        "Asf1" => Family::Asf1,
        "Asf2" => Family::Asf2,
        "Asf3" => Family::Asf3,
        "City" => Family::City,
        "Grav" => Family::Grav,
        _ => Family::Mud,
    }
}

fn dump_strips(path: &str, strips: &[RoadStrip], net: &naves_core::roadplace::PlacedNetwork) {
    let mut out = String::new();
    for st in strips {
        let first = &net.roads[st.slots[0]];
        let mut ids: Vec<String> = Vec::new();
        for s in &st.slots {
            let id = net.roads[*s].slot.stretch.clone();
            if ids.last() != Some(&id) {
                ids.push(id);
            }
        }
        writeln!(
            out,
            "S\t{:?}\t{}\t{}\t{}",
            first.family,
            first.drivable,
            st.half_width,
            ids.join(",")
        )
        .unwrap();
        for (i, j) in st.joints.iter().enumerate() {
            writeln!(
                out,
                "J\t{}\t{}\t{}\t{}\t{}",
                j.0,
                j.1,
                j.2,
                st.bridge.get(i).copied().unwrap_or(false),
                st.junction.get(i).copied().unwrap_or(false)
            )
            .unwrap();
        }
    }
    std::fs::write(path, out).expect("write dump");
}

/// The graph for the junction study: crossings with their live arms, and the joins without a junction piece.
fn dump_graph(
    path: &str,
    graph: &naves_core::roadnet::RoadGraph,
    net: &naves_core::roadplace::PlacedNetwork,
) {
    let mut out = String::new();
    for c in &graph.crossings {
        let arms: Vec<String> = c
            .stretches
            .iter()
            .filter_map(|i| graph.stretches.get(*i))
            .filter(|s| !s.tunnel)
            .map(|s| {
                format!(
                    "{}@{}{}",
                    s.id,
                    if s.start == c.node { 's' } else { 'e' },
                    if s.start == s.end { 'r' } else { '-' }
                )
            })
            .collect();
        writeln!(
            out,
            "C\t{}\t{}\t{}\t{}",
            c.node,
            c.point.0,
            c.point.1,
            arms.join(",")
        )
        .unwrap();
    }
    for p in &net.joins_without_junction {
        writeln!(out, "P\t{}\t{}", p.0, p.1).unwrap();
    }
    std::fs::write(path, out).expect("write graph dump");
}

fn load_strips(path: &str) -> Vec<DStrip> {
    let text = std::fs::read_to_string(path).expect("read dump");
    let mut out: Vec<DStrip> = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] == "S" {
            out.push(DStrip {
                fam: parse_family(f[1]),
                drivable: f[2] == "true",
                half_width: f[3].parse().unwrap(),
                ids: f[4].split(',').map(str::to_string).collect(),
                joints: Vec::new(),
                bridge: Vec::new(),
                junction: Vec::new(),
            });
        } else if let Some(s) = out.last_mut() {
            s.joints.push((
                f[1].parse().unwrap(),
                f[2].parse().unwrap(),
                f[3].parse().unwrap(),
            ));
            s.bridge.push(f[4] == "true");
            s.junction.push(f[5] == "true");
        }
    }
    out
}

/// Copy of `profile::meeting_on`: where a point lies on one strip's surface (distance, segment, t).
fn meeting_on_d(
    strip: &DStrip,
    at: (f64, f64),
    reach: f64,
    junction_reach: f64,
) -> Option<(f64, usize, f64)> {
    let mut best: Option<(f64, usize, f64)> = None;
    for segment in 0..strip.joints.len().saturating_sub(1) {
        if strip.bridge.get(segment).copied().unwrap_or(false) {
            continue;
        }
        let (from, to) = (strip.joints[segment], strip.joints[segment + 1]);
        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
        let length2 = dx * dx + dy * dy;
        let t = if length2 > 0.0 {
            (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let junction = strip.junction.get(segment).copied().unwrap_or(false);
        let distance = if junction {
            let (mx, my) = (
                f64::midpoint(from.0, to.0) - at.0,
                f64::midpoint(from.1, to.1) - at.1,
            );
            (mx.hypot(my) - length2.sqrt() / 2.0).max(0.0)
        } else {
            (at.0 - from.0 - t * dx).hypot(at.1 - from.1 - t * dy)
        };
        let allowed = if junction {
            junction_reach
        } else {
            strip.half_width + reach
        };
        if distance <= allowed && best.is_none_or(|(found, _, _)| distance < found) {
            best = Some((distance, segment, t));
        }
    }
    best
}

fn meeting_h(strip: &DStrip, segment: usize, t: f64) -> f64 {
    let (a, b) = (strip.joints[segment], strip.joints[segment + 1]);
    a.2 + t * (b.2 - a.2)
}

/// Copy of `profile::meeting`: the nearest earlier strip whose surface the point lies on.
fn meeting_d(strips: &[DStrip], upto: usize, at: (f64, f64)) -> Option<(usize, usize, f64)> {
    let mut best: Option<(f64, usize, usize, f64)> = None;
    for (owner, s) in strips.iter().enumerate().take(upto) {
        if let Some((d, seg, t)) = meeting_on_d(s, at, 1.0, 1.0)
            && best.is_none_or(|(found, _, _, _)| d < found)
        {
            best = Some((d, owner, seg, t));
        }
    }
    best.map(|(_, o, s, t)| (o, s, t))
}

struct Line {
    strip: usize,
    ids: Vec<String>,
    fam: Family,
    drivable: bool,
    len: f64,
    jp: Vec<(f64, f64)>,
    cs: Vec<f64>,
    jbridge: Vec<bool>,
    pts: Vec<(f64, f64)>,
    st: Vec<f64>,
    dl: Vec<f64>,
    pin: Vec<Option<f64>>,
    bground: Vec<Option<f64>>,
    owner: [Option<usize>; 2],
    nrm: Vec<(f64, f64)>,
    kappa: Vec<f64>,
}

impl Line {
    fn track(&self) -> bool {
        matches!(self.fam, Family::Grav | Family::Mud)
    }
    fn group(&self) -> String {
        match self.fam {
            Family::Grav => "Grav".into(),
            Family::Mud if self.drivable => "Mud drivable".into(),
            Family::Mud => "Mud not drivable".into(),
            f => format!("{f:?}"),
        }
    }
    fn n(&self) -> usize {
        self.pts.len()
    }
    /// Position and segment at a station along the joints' polyline.
    fn at_station(&self, s: f64) -> ((f64, f64), usize) {
        let nseg = self.jp.len() - 1;
        let seg = (self.cs.partition_point(|c| *c <= s).saturating_sub(1)).min(nseg - 1);
        let l = self.cs[seg + 1] - self.cs[seg];
        let t = if l > 0.0 {
            ((s - self.cs[seg]) / l).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (a, b) = (self.jp[seg], self.jp[seg + 1]);
        ((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)), seg)
    }
    fn ground(&self, base: &Heightmap, i: usize, o: f64) -> f64 {
        self.bground[i].unwrap_or_else(|| {
            let p = self.shifted(i, o);
            f64::from(base.sample(p.0, p.1).height)
        })
    }
    fn shifted(&self, i: usize, o: f64) -> (f64, f64) {
        (
            self.pts[i].0 + o * self.nrm[i].0,
            self.pts[i].1 + o * self.nrm[i].1,
        )
    }
    fn length_with(&self, off: &[f64]) -> f64 {
        (1..self.n())
            .map(|i| {
                let (a, b) = (self.shifted(i - 1, off[i - 1]), self.shifted(i, off[i]));
                (b.0 - a.0).hypot(b.1 - a.1)
            })
            .sum()
    }
}

fn build_lines(strips: &[DStrip], base: &Heightmap) -> Vec<Line> {
    let cell = f64::from(base.cell_m);
    let mut lines = Vec::new();
    for (si, s) in strips.iter().enumerate() {
        if s.joints.len() < 2 {
            continue;
        }
        let jp: Vec<(f64, f64)> = s.joints.iter().map(|j| (j.0, j.1)).collect();
        let mut cs = vec![0.0];
        for w in jp.windows(2) {
            let last = *cs.last().unwrap();
            cs.push(last + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
        }
        let len = *cs.last().unwrap();
        if len < 2.0 {
            continue;
        }
        let mut line = Line {
            strip: si,
            ids: s.ids.clone(),
            fam: s.fam,
            drivable: s.drivable,
            len,
            jp,
            cs,
            jbridge: (0..s.joints.len() - 1)
                .map(|i| s.bridge.get(i).copied().unwrap_or(false))
                .collect(),
            pts: Vec::new(),
            st: Vec::new(),
            dl: Vec::new(),
            pin: Vec::new(),
            bground: Vec::new(),
            owner: [None, None],
            nrm: Vec::new(),
            kappa: Vec::new(),
        };
        let n = ((len / SAMPLE_STEP_M).round() as usize).max(1);
        let mut inside_bridge = Vec::new();
        for k in 0..=n {
            let sta = len * k as f64 / n as f64;
            let (p, seg) = line.at_station(sta);
            line.pts.push(p);
            line.st.push(sta);
            let inside =
                line.jbridge[seg] && sta > line.cs[seg] + 0.01 && sta < line.cs[seg + 1] - 0.01;
            inside_bridge.push(inside);
        }
        line.dl = line
            .pts
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .collect();
        // ground under a deck: the line between the abutments
        let g0: Vec<f64> = line
            .pts
            .iter()
            .map(|p| f64::from(base.sample(p.0, p.1).height))
            .collect();
        line.bground = vec![None; n + 1];
        let mut i = 0;
        while i <= n {
            if inside_bridge[i] {
                let a = i;
                while i <= n && inside_bridge[i] {
                    i += 1;
                }
                let b = i - 1;
                let left = a.saturating_sub(1);
                let right = (b + 1).min(n);
                for k in a..=b {
                    let f =
                        (line.st[k] - line.st[left]) / (line.st[right] - line.st[left]).max(0.1);
                    line.bground[k] = Some(g0[left] + f * (g0[right] - g0[left]));
                }
            } else {
                i += 1;
            }
        }
        // pins, as `end_joins` does it
        line.pin = vec![None; n + 1];
        for end in 0..2 {
            let (point, idx0) = if end == 0 {
                (line.jp[0], 0)
            } else {
                (line.jp[line.jp.len() - 1], n)
            };
            let Some((owner, seg, t)) = meeting_d(strips, si, point) else {
                continue;
            };
            line.owner[end] = Some(owner);
            let os = &strips[owner];
            let reach = os.half_width + 1.5 * cell;
            let _ = (seg, t);
            for step in 0..8 {
                let k = if end == 0 {
                    idx0 + step
                } else {
                    match idx0.checked_sub(step) {
                        Some(k) => k,
                        None => break,
                    }
                };
                if k > n {
                    break;
                }
                let Some((_, sg, tt)) = meeting_on_d(os, line.pts[k], reach, reach) else {
                    break;
                };
                line.pin[k] = Some(meeting_h(os, sg, tt));
            }
        }
        // normals and curvature
        for i in 0..=n {
            let (a, b) = (line.pts[i.saturating_sub(1)], line.pts[(i + 1).min(n)]);
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l = dx.hypot(dy).max(1e-9);
            line.nrm.push((-dy / l, dx / l));
            let k = if i == 0 || i == n {
                0.0
            } else {
                let (p0, p1, p2) = (line.pts[i - 1], line.pts[i], line.pts[i + 1]);
                let h1 = (p1.1 - p0.1).atan2(p1.0 - p0.0);
                let h2 = (p2.1 - p1.1).atan2(p2.0 - p1.0);
                let d = (h2 - h1 + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI;
                d / (0.5 * (line.dl[i - 1] + line.dl[i])).max(0.5)
            };
            line.kappa.push(k);
        }
        lines.push(line);
    }
    lines
}

/// Whether heights within `d` of `g` (pinned samples exactly at their pin) exist under `grade`; the index
/// where it first fails, if it does.
fn fails_at(g: &[f64], dl: &[f64], pin: &[Option<f64>], grade: f64, d: f64) -> Option<usize> {
    let iv = |i: usize| match pin[i] {
        Some(h) => (h, h),
        None => (g[i] - d, g[i] + d),
    };
    let (mut lo, mut hi) = iv(0);
    for i in 1..g.len() {
        let step = grade * dl[i - 1];
        lo -= step;
        hi += step;
        let (a, b) = iv(i);
        lo = lo.max(a);
        hi = hi.min(b);
        if lo > hi + 1e-9 {
            return Some(i);
        }
    }
    None
}

/// The smallest `d`, where it fails first (just below it), and whether pins at the far end had to be dropped
/// because the pins cannot be joined at this grade at all.
fn min_d(g: &[f64], dl: &[f64], pin: &[Option<f64>], grade: f64) -> (f64, Option<usize>, bool) {
    let mut pins = pin.to_vec();
    let mut relaxed = false;
    if fails_at(g, dl, &pins, grade, 400.0).is_some() {
        relaxed = true;
        let half = g.len() / 2;
        for p in pins.iter_mut().skip(half) {
            *p = None;
        }
        if fails_at(g, dl, &pins, grade, 400.0).is_some() {
            pins = vec![None; g.len()];
        }
    }
    let (mut lo, mut hi) = (0.0_f64, 400.0_f64);
    if fails_at(g, dl, &pins, grade, 0.0).is_none() {
        return (0.0, None, relaxed);
    }
    for _ in 0..34 {
        let mid = 0.5 * (lo + hi);
        if fails_at(g, dl, &pins, grade, mid).is_some() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (hi, fails_at(g, dl, &pins, grade, lo), relaxed)
}

fn grounds(line: &Line, base: &Heightmap, off: &[f64]) -> Vec<f64> {
    (0..line.n())
        .map(|i| line.ground(base, i, off[i]))
        .collect()
}

/// Shifted-line segment lengths.
fn shifted_dl(line: &Line, off: &[f64]) -> Vec<f64> {
    (1..line.n())
        .map(|i| {
            let (a, b) = (line.shifted(i - 1, off[i - 1]), line.shifted(i, off[i]));
            (b.0 - a.0).hypot(b.1 - a.1).max(0.5)
        })
        .collect()
}

fn d_of(line: &Line, base: &Heightmap, off: &[f64], grade: f64) -> (f64, Option<usize>, bool) {
    let g = grounds(line, base, off);
    let dl = shifted_dl(line, off);
    min_d(&g, &dl, &line.pin, grade)
}

/// The same with the pins ignored: what the ground alone along the line allows.
fn d_free(line: &Line, base: &Heightmap, off: &[f64], grade: f64) -> (f64, Option<usize>, bool) {
    let g = grounds(line, base, off);
    let dl = shifted_dl(line, off);
    min_d(&g, &dl, &vec![None; g.len()], grade)
}

/// Km of the line where the ground itself along the line is steeper than `grade` over 12.5 m.
fn steep_km(line: &Line, base: &Heightmap, grade: f64) -> f64 {
    let step = 2.5;
    let n = (line.len / step).floor() as usize;
    let mut g = Vec::new();
    let mut br = Vec::new();
    for k in 0..=n {
        let sta = k as f64 * step;
        let (p, seg) = line.at_station(sta);
        g.push(f64::from(base.sample(p.0, p.1).height));
        br.push(line.jbridge[seg]);
    }
    let mut m = 0.0;
    for k in 0..g.len().saturating_sub(5) {
        if br[k..=k + 5].iter().any(|b| *b) {
            continue;
        }
        if (g[k + 5] - g[k]).abs() / 12.5 > grade {
            m += step;
        }
    }
    m
}

// ---- the clash index: every strip as segments with planned heights

struct Sg {
    strip: usize,
    a: (f64, f64, f64),
    b: (f64, f64, f64),
    sa: f64,
    sb: f64,
}

struct Ctx {
    segs: Vec<Sg>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

const GRID_M: f64 = 25.0;
const CLASH_M: f64 = 20.0;
const CLASH_DH: f64 = 1.0;

fn planned_at(line: &Line, strips: &[DStrip], s: f64) -> f64 {
    let (_, seg) = line.at_station(s);
    let l = line.cs[seg + 1] - line.cs[seg];
    let t = if l > 0.0 { (s - line.cs[seg]) / l } else { 0.0 };
    let (a, b) = (
        strips[line.strip].joints[seg],
        strips[line.strip].joints[seg + 1],
    );
    a.2 + t.clamp(0.0, 1.0) * (b.2 - a.2)
}

/// Segments of everything: tracks from their samples (shifted by `off` when given), the rest from joints.
fn build_ctx(lines: &[Line], strips: &[DStrip], off: Option<&[Vec<f64>]>) -> Ctx {
    let mut segs = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        if line.track() {
            let zero;
            let o: &[f64] = if let Some(all) = off {
                &all[li]
            } else {
                zero = vec![0.0; line.n()];
                &zero
            };
            for i in 1..line.n() {
                let mid = 0.5 * (line.st[i - 1] + line.st[i]);
                let (_, seg) = line.at_station(mid);
                if line.jbridge[seg] {
                    continue;
                }
                let (pa, pb) = (line.shifted(i - 1, o[i - 1]), line.shifted(i, o[i]));
                segs.push(Sg {
                    strip: line.strip,
                    a: (pa.0, pa.1, planned_at(line, strips, line.st[i - 1])),
                    b: (pb.0, pb.1, planned_at(line, strips, line.st[i])),
                    sa: line.st[i - 1],
                    sb: line.st[i],
                });
            }
        } else {
            let s = &strips[line.strip];
            for i in 0..s.joints.len() - 1 {
                if s.bridge.get(i).copied().unwrap_or(false) {
                    continue;
                }
                segs.push(Sg {
                    strip: line.strip,
                    a: s.joints[i],
                    b: s.joints[i + 1],
                    sa: line.cs[i],
                    sb: line.cs[i + 1],
                });
            }
        }
    }
    let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
    for (k, sg) in segs.iter().enumerate() {
        let cell = |v: f64| (v / GRID_M).floor() as i32;
        let (x0, x1) = (cell(sg.a.0.min(sg.b.0)), cell(sg.a.0.max(sg.b.0)));
        let (y0, y1) = (cell(sg.a.1.min(sg.b.1)), cell(sg.a.1.max(sg.b.1)));
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                grid.entry((cx, cy)).or_default().push(k as u32);
            }
        }
    }
    Ctx { segs, grid }
}

/// Whether a point of `line` (sample `i`, ground `g`) is within 20 m of another road whose planned height
/// differs from the ground there by more than 1 m. The road a chain end is pinned to, near that end, and
/// the chain's own legs within 60 m along the line, do not count.
fn clash_at(ctx: &Ctx, line: &Line, i: usize, p: (f64, f64), g: f64) -> bool {
    let (cx, cy) = ((p.0 / GRID_M).floor() as i32, (p.1 / GRID_M).floor() as i32);
    for dx in -1..=1 {
        for dy in -1..=1 {
            let Some(list) = ctx.grid.get(&(cx + dx, cy + dy)) else {
                continue;
            };
            for k in list {
                let sg = &ctx.segs[*k as usize];
                let (d, t) = seg_dist(p, (sg.a.0, sg.a.1), (sg.b.0, sg.b.1));
                if d > CLASH_M {
                    continue;
                }
                let s_there = sg.sa + t * (sg.sb - sg.sa);
                if sg.strip == line.strip && (s_there - line.st[i]).abs() < 60.0 {
                    continue;
                }
                if line.owner[0] == Some(sg.strip) && line.st[i] < 40.0 {
                    continue;
                }
                if line.owner[1] == Some(sg.strip) && line.len - line.st[i] < 40.0 {
                    continue;
                }
                let h = sg.a.2 + t * (sg.b.2 - sg.a.2);
                if (g - h).abs() > CLASH_DH {
                    return true;
                }
            }
        }
    }
    false
}

struct Cfg {
    grade: f64,
    reach: f64,
    slope: f64,
    lambda: f64,
    clash_pen: f64,
}

/// The offsets that minimise the grade excess over the ceiling (integrated over length), plus a small cost
/// per metre of offset, plus (when `ctx` is given) a cost per metre for being in a clash.
fn shift_dp(line: &Line, base: &Heightmap, map_m: f64, cfg: &Cfg, ctx: Option<&Ctx>) -> Vec<f64> {
    let n = line.n();
    let m = (cfg.reach / OFFSET_STEP_M).round() as i64;
    let k = (2 * m + 1) as usize;
    let off_of = |a: usize| (a as i64 - m) as f64 * OFFSET_STEP_M;
    let mut g = vec![0.0; n * k];
    let mut ok = vec![false; n * k];
    let mut un = vec![0.0; n * k];
    for i in 0..n {
        let lw = match (i.checked_sub(1).map(|j| line.dl[j]), line.dl.get(i)) {
            (Some(a), Some(b)) => 0.5 * (a + b),
            (Some(a), None) => a,
            (None, Some(b)) => *b,
            _ => 0.0,
        };
        let fixed = line.pin[i].is_some() || line.bground[i].is_some();
        for a in 0..k {
            let o = off_of(a);
            let idx = i * k + a;
            if fixed && a as i64 != m {
                continue;
            }
            let p = line.shifted(i, o);
            if p.0 < 0.0 || p.1 < 0.0 || p.0 > map_m || p.1 > map_m {
                continue;
            }
            // do not cut through the inside of a bend
            let kap = line.kappa[i];
            if kap.abs() > 1e-9 && o * kap.signum() > 0.85 / kap.abs() {
                continue;
            }
            ok[idx] = true;
            let gh = line.ground(base, i, o);
            g[idx] = line.pin[i].unwrap_or(gh);
            un[idx] = cfg.lambda * o.abs() * lw;
            if let (Some(ctx), true) = (ctx, cfg.clash_pen > 0.0)
                && !fixed
                && clash_at(ctx, line, i, p, gh)
            {
                un[idx] += cfg.clash_pen * lw;
            }
        }
    }
    let inf = f64::INFINITY;
    let mut cost = vec![inf; n * k];
    let mut from = vec![0_i8; n * k];
    for a in 0..k {
        if ok[a] {
            cost[a] = un[a];
        }
    }
    for i in 1..n {
        let dmax = ((cfg.slope * line.dl[i - 1] / OFFSET_STEP_M + 1e-9).floor() as i64).max(1);
        for b in 0..k {
            let bi = i * k + b;
            if !ok[bi] {
                continue;
            }
            let pb = line.shifted(i, off_of(b));
            let mut best = inf;
            let mut best_a = 0_i8;
            let lo = (b as i64 - dmax).max(0);
            let hi = (b as i64 + dmax).min(k as i64 - 1);
            for a in lo..=hi {
                let ai = (i - 1) * k + a as usize;
                if !ok[ai] || cost[ai] == inf {
                    continue;
                }
                let pa = line.shifted(i - 1, off_of(a as usize));
                let l = (pb.0 - pa.0).hypot(pb.1 - pa.1).max(0.5);
                let excess = (((g[bi] - g[ai]).abs() / l) - cfg.grade).max(0.0) * l;
                let c = cost[ai] + excess;
                if c < best {
                    best = c;
                    best_a = (a - b as i64) as i8;
                }
            }
            if best < inf {
                cost[bi] = best + un[bi];
                from[bi] = best_a;
            }
        }
    }
    let last = (n - 1) * k;
    let mut b = (0..k)
        .filter(|a| cost[last + a] < inf)
        .min_by(|x, y| cost[last + x].total_cmp(&cost[last + y]))
        .unwrap_or(m as usize);
    let mut off = vec![0.0; n];
    for i in (0..n).rev() {
        off[i] = off_of(b);
        if i > 0 {
            b = (b as i64 + i64::from(from[i * k + b])) as usize;
        }
    }
    off
}

fn quant(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    pct(v, q)
}

/// "chains/km | share of chains and of km at d <= 0.5, 2, 5 and > 10 | median, p90".
fn bucket_line(items: &[(f64, f64)]) -> String {
    let n = items.len();
    let km: f64 = items.iter().map(|x| x.0).sum::<f64>() / 1000.0;
    let share = |f: &dyn Fn(f64) -> bool| {
        let c = items.iter().filter(|x| f(x.1)).count() as f64 / n.max(1) as f64 * 100.0;
        let k = items.iter().filter(|x| f(x.1)).map(|x| x.0).sum::<f64>() / 1000.0 / km.max(1e-9)
            * 100.0;
        format!("{c:3.0}/{k:3.0}")
    };
    let mut d: Vec<f64> = items
        .iter()
        .map(|x| x.1)
        .filter(|x| x.is_finite())
        .collect();
    format!(
        "{n:4} ch {km:6.1} km | <=0.5 {} | <=2 {} | <=5 {} | >10 {} | p50 {:5.2} p90 {:6.2}",
        share(&|d| d <= 0.5),
        share(&|d| d <= 2.0),
        share(&|d| d <= 5.0),
        share(&|d| d > 10.0),
        quant(&mut d, 0.5),
        quant(&mut d, 0.9)
    )
}

const KLINGER: &str = "158289388:259090646-278531300";

fn shift_study(strips: &[DStrip], base: &Heightmap) {
    println!(
        "\n######## MODE 1: pins as the build has them (a pinned end is held at the other road's planned height)"
    );
    shift_mode(strips, base, false);
    println!(
        "\n######## MODE 2: pins ignored (what the ground alone along the line allows; the offset is still 0 at a pinned end)"
    );
    shift_mode(strips, base, true);
}

fn shift_mode(strips: &[DStrip], base: &Heightmap, free: bool) {
    let d_of = if free { d_free } else { d_of };
    let t0 = Instant::now();
    let map_m = f64::from(base.size - 1) * f64::from(base.cell_m);
    let lines = build_lines(strips, base);
    let zero: Vec<Vec<f64>> = lines.iter().map(|l| vec![0.0; l.n()]).collect();
    let tracks: Vec<usize> = (0..lines.len()).filter(|i| lines[*i].track()).collect();
    println!(
        "\n#### SHIFT STUDY: size {}, cell {} m, {} strips, {} lines, {} tracks ({:.1} km), sample {} m",
        base.size,
        base.cell_m,
        strips.len(),
        lines.len(),
        tracks.len(),
        tracks.iter().map(|i| lines[*i].len).sum::<f64>() / 1000.0,
        SAMPLE_STEP_M
    );
    let klinger: Option<usize> = tracks
        .iter()
        .copied()
        .find(|i| lines[*i].ids.iter().any(|id| id == KLINGER));
    println!(
        "Klingerweg strip: {:?}",
        klinger.map(|i| (
            lines[i].strip,
            lines[i].len,
            lines[i].pin.iter().filter(|p| p.is_some()).count()
        ))
    );
    let pinned_ends = tracks
        .iter()
        .map(|i| lines[*i].owner.iter().filter(|o| o.is_some()).count())
        .sum::<usize>();
    println!(
        "track ends pinned to another road: {pinned_ends} of {} ({} free)",
        tracks.len() * 2,
        tracks.len() * 2 - pinned_ends
    );

    if !free {
        println!(
            "\n#### pinned track ends: how far is the other road's planned height from the ground there?"
        );
        let mut by_owner: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        for &i in &tracks {
            let l = &lines[i];
            for (end, k) in [(0, 0), (1, l.n() - 1)] {
                if let (Some(ow), Some(h)) = (l.owner[end], l.pin[k]) {
                    let g = f64::from(base.sample(l.pts[k].0, l.pts[k].1).height);
                    by_owner
                        .entry(format!("{:?}", strips[ow].fam))
                        .or_default()
                        .push((h - g).abs());
                }
            }
        }
        for (fam, mut v) in by_owner {
            let n = v.len();
            let over = |t: f64| v.iter().filter(|x| **x > t).count();
            println!(
                "  pinned to {fam:5} {n:4} ends | |planned - ground| > 1 m: {}, > 2 m: {}, > 5 m: {}, > 10 m: {} | p50 {:.2} p90 {:.2} m",
                over(1.0),
                over(2.0),
                over(5.0),
                over(10.0),
                quant(&mut v, 0.5),
                quant(&mut v, 0.9)
            );
        }
    }
    // ---- A
    println!(
        "\n#### A. smallest d (m) for a profile within d of the ground, at the grade ceiling G"
    );
    println!(
        "columns: chains, km | share of chains/share of km with d <= 0.5, <= 2, <= 5 and > 10 m | median and p90 of d"
    );
    let ceilings: [f64; 6] = [0.18, 0.25, 0.30, 0.36, 0.45, 0.60];
    let groups = ["Grav", "Mud drivable", "Mud not drivable"];
    let mut d0: BTreeMap<(usize, u32), (f64, Option<usize>, bool)> = BTreeMap::new();
    for g in ceilings {
        let key = (g * 100.0).round() as u32;
        for (li, line) in lines.iter().enumerate() {
            if line.track() {
                d0.insert((li, key), d_of(line, base, &zero[li], g));
            }
        }
    }
    for g in ceilings {
        let key = (g * 100.0).round() as u32;
        for grp in groups {
            let sel: Vec<usize> = tracks
                .iter()
                .copied()
                .filter(|i| lines[*i].group() == grp)
                .collect();
            let items: Vec<(f64, f64)> = sel
                .iter()
                .map(|i| (lines[*i].len, d0[&(*i, key)].0))
                .collect();
            let steep: f64 = sel
                .iter()
                .map(|i| steep_km(&lines[*i], base, g))
                .sum::<f64>()
                / 1000.0;
            let relaxed = sel.iter().filter(|i| d0[&(**i, key)].2).count();
            println!(
                "A G={:2.0}% {grp:17} {} | ground steeper than G over 12.5 m: {steep:5.2} km | pins not joinable: {relaxed}",
                g * 100.0,
                bucket_line(&items)
            );
        }
    }
    println!("public roads at their own ceiling (for scale):");
    for fam in [Family::Asf1, Family::Asf2, Family::Asf3, Family::City] {
        let g = naves_core::limits::grade_ceiling(fam);
        let sel: Vec<&Line> = lines.iter().filter(|l| l.fam == fam).collect();
        let items: Vec<(f64, f64)> = sel
            .iter()
            .map(|l| (l.len, d_of(l, base, &vec![0.0; l.n()], g).0))
            .collect();
        let steep: f64 = sel.iter().map(|l| steep_km(l, base, g)).sum::<f64>() / 1000.0;
        println!(
            "A public {fam:?} G={:4.1}% {} | ground steeper: {steep:5.2} km",
            g * 100.0,
            bucket_line(&items)
        );
    }
    println!("track families at their own ceilings (Grav 27 %, Mud 25.2 %), today's rule:");
    for (grp, g) in [
        ("Grav", 0.27),
        ("Mud drivable", 0.252),
        ("Mud not drivable", 0.252),
    ] {
        let items: Vec<(f64, f64)> = tracks
            .iter()
            .filter(|i| lines[**i].group() == grp)
            .map(|i| (lines[*i].len, d_of(&lines[*i], base, &zero[*i], g).0))
            .collect();
        println!(
            "A today {grp:17} G={:4.1}% {}",
            g * 100.0,
            bucket_line(&items)
        );
    }
    let worst_table = |key: u32, g: f64, extra: &dyn Fn(usize) -> String| {
        let mut order: Vec<usize> = tracks.clone();
        order.sort_by(|a, b| d0[&(*b, key)].0.total_cmp(&d0[&(*a, key)].0));
        for i in order.iter().take(20) {
            let l = &lines[*i];
            let (d, at, _) = d0[&(*i, key)];
            let g0 = grounds(l, base, &zero[*i]);
            let mean = (g0[g0.len() - 1] - g0[0]).abs() / l.len * 100.0;
            let spot = at.map_or("-".to_string(), |k| {
                format!("({:.0},{:.0})", l.pts[k].0, l.pts[k].1)
            });
            println!(
                "  strip {:4} {:5} {:16} {:6.0} m  mean grade {:4.1} %  d {:6.2} m at {spot}{}  [{}]",
                l.strip,
                format!("{:?}", l.fam),
                l.group(),
                l.len,
                mean,
                d,
                extra(*i),
                l.ids[0]
            );
        }
        let _ = g;
    };
    for g in [0.25_f64, 0.36] {
        let key = (g * 100.0).round() as u32;
        println!("twenty worst track chains at G = {:.0} %:", g * 100.0);
        worst_table(key, g, &|_| String::new());
    }
    if let Some(i) = klinger {
        let l = &lines[i];
        print!(
            "Klingerweg (strip {}, {:.0} m, {}): d at G =",
            l.strip,
            l.len,
            l.group()
        );
        for g in ceilings {
            let key = (g * 100.0).round() as u32;
            let (d, at, relaxed) = d0[&(i, key)];
            print!(
                " {:.0}%: {d:.2} m{}{}",
                g * 100.0,
                at.map_or(String::new(), |k| format!(
                    " at ({:.0},{:.0})",
                    l.pts[k].0, l.pts[k].1
                )),
                if relaxed { " (pins relaxed)" } else { "" }
            );
        }
        println!();
        let g0 = grounds(l, base, &zero[i]);
        println!(
            "  ends: ground {:.1} m -> {:.1} m, pins {:?} / {:?}",
            g0[0],
            g0[g0.len() - 1],
            l.pin[0],
            l.pin[l.n() - 1]
        );
    }
    println!("A done {:.1} s", t0.elapsed().as_secs_f64());

    // ---- B
    println!(
        "\n#### B. what a sideways shift buys (offset step {OFFSET_STEP_M} m, cost 0.001 m per m of offset and length)"
    );
    struct Run {
        g: f64,
        r: f64,
        slope: f64,
        pen: bool,
        off: Vec<Vec<f64>>,
        d: BTreeMap<usize, (f64, Option<usize>, bool)>,
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut configs: Vec<(f64, f64, f64, bool)> = Vec::new();
    for g in [0.25_f64, 0.36] {
        for r in [5.0, 10.0, 15.0] {
            configs.push((g, r, 0.25, false));
        }
        configs.push((g, 10.0, 0.5, false));
    }
    // the clash variant (C): after the plain runs, which it needs the clash index of
    let ctx0 = build_ctx(&lines, strips, None);
    configs.push((0.36, 10.0, 0.25, true));
    for (g, r, slope, pen) in configs {
        let cfg = Cfg {
            grade: g,
            reach: r,
            slope,
            lambda: 0.001,
            clash_pen: if pen { 0.25 } else { 0.0 },
        };
        let mut off = vec![Vec::new(); lines.len()];
        let mut d = BTreeMap::new();
        for &i in &tracks {
            let o = shift_dp(
                &lines[i],
                base,
                map_m,
                &cfg,
                if pen { Some(&ctx0) } else { None },
            );
            d.insert(i, d_of(&lines[i], base, &o, g));
            off[i] = o;
        }
        runs.push(Run {
            g,
            r,
            slope,
            pen,
            off,
            d,
        });
        eprintln!(
            "run G {g} R {r} slope {slope} pen {pen} done {:.1} s",
            t0.elapsed().as_secs_f64()
        );
    }
    for run in runs.iter().filter(|r| !r.pen) {
        let key = (run.g * 100.0).round() as u32;
        println!(
            "\nB G={:.0}% R={:.0} m slope {} m/m",
            run.g * 100.0,
            run.r,
            run.slope
        );
        for grp in groups {
            let sel: Vec<usize> = tracks
                .iter()
                .copied()
                .filter(|i| lines[*i].group() == grp)
                .collect();
            let before: Vec<(f64, f64)> = sel
                .iter()
                .map(|i| (lines[*i].len, d0[&(*i, key)].0))
                .collect();
            let after: Vec<(f64, f64)> = sel.iter().map(|i| (lines[*i].len, run.d[i].0)).collect();
            let mut imp: Vec<f64> = sel
                .iter()
                .filter(|i| d0[&(**i, key)].0.is_finite() && run.d[i].0.is_finite())
                .map(|i| d0[&(*i, key)].0 - run.d[i].0)
                .collect();
            let n1 = imp.iter().filter(|x| **x > 1.0).count();
            let n5 = imp.iter().filter(|x| **x > 5.0).count();
            let mut abs_o: Vec<f64> = sel
                .iter()
                .flat_map(|i| run.off[*i].iter().map(|o| o.abs()))
                .collect();
            let half = abs_o.iter().filter(|o| **o > run.r / 2.0 + 1e-9).count() as f64
                / abs_o.len().max(1) as f64
                * 100.0;
            let l0: f64 = sel.iter().map(|i| lines[*i].len).sum();
            let l1: f64 = sel
                .iter()
                .map(|i| lines[*i].length_with(&run.off[*i]))
                .sum();
            println!("  {grp:17} before {}", bucket_line(&before));
            println!("  {:17} after  {}", "", bucket_line(&after));
            println!(
                "  {:17} improvement p50 {:5.2} p90 {:6.2} m; >1 m: {n1}, >5 m: {n5} | |offset| p50 {:4.2} p90 {:5.2} m, share > R/2: {half:4.1} % | length {:+.2} %",
                "",
                quant(&mut imp, 0.5),
                quant(&mut imp, 0.9),
                quant(&mut abs_o, 0.5),
                quant(&mut abs_o, 0.9),
                (l1 / l0 - 1.0) * 100.0
            );
        }
    }
    if let Some(i) = klinger {
        println!("\nKlingerweg after the shift:");
        println!(
            "  d unshifted: G25 {:.2}, G36 {:.2}",
            d0[&(i, 25)].0,
            d0[&(i, 36)].0
        );
        for run in runs.iter().filter(|r| !r.pen) {
            let o = &run.off[i];
            let l1 = lines[i].length_with(o);
            let mx = o.iter().fold(0.0_f64, |a, b| a.max(b.abs()));
            println!(
                "  G={:.0}% R={:2.0} slope {}: d {:.2} m, max |offset| {mx:.2} m, length {:.0} -> {l1:.0} m",
                run.g * 100.0,
                run.r,
                run.slope,
                run.d[&i].0,
                lines[i].len
            );
        }
    }
    for g in [0.25_f64, 0.36] {
        let key = (g * 100.0).round() as u32;
        println!(
            "\ntwenty worst chains of A at G = {:.0} %, after the shift (d unshifted | R=5 | 10 | 15):",
            g * 100.0
        );
        let cols: Vec<&Run> = [5.0, 10.0, 15.0]
            .iter()
            .filter_map(|r| {
                runs.iter()
                    .find(|x| !x.pen && x.slope == 0.25 && x.g == g && x.r == *r)
            })
            .collect();
        worst_table(key, g, &|i| {
            let mut s = String::from("  shifted:");
            for c in &cols {
                let _ = write!(s, " {:.2}", c.d[&i].0);
            }
            s
        });
    }

    // ---- C
    println!(
        "\n#### C. clashes: track samples within {CLASH_M} m of another road (centre to centre) whose planned height differs from the ground by more than {CLASH_DH} m"
    );
    println!(
        "(counted per 5 m sample; the road a chain end is pinned to, near that end, and the chain's own legs within 60 m along the line do not count; other tracks: shifted too when shifted)"
    );
    let count_clash = |off: Option<&[Vec<f64>]>| -> BTreeMap<String, (usize, f64)> {
        let ctx = build_ctx(&lines, strips, off);
        let mut out: BTreeMap<String, (usize, f64)> = BTreeMap::new();
        for &i in &tracks {
            let l = &lines[i];
            let o: Vec<f64> = off.map_or_else(|| vec![0.0; l.n()], |a| a[i].clone());
            for k in 0..l.n() {
                if l.pin[k].is_some() || l.bground[k].is_some() {
                    continue;
                }
                let p = l.shifted(k, o[k]);
                let g = l.ground(base, k, o[k]);
                if clash_at(&ctx, l, k, p, g) {
                    let w = match (k.checked_sub(1).map(|j| l.dl[j]), l.dl.get(k)) {
                        (Some(a), Some(b)) => 0.5 * (a + b),
                        (Some(a), None) => a,
                        (None, Some(b)) => *b,
                        _ => 0.0,
                    };
                    let e = out.entry(l.group()).or_default();
                    e.0 += 1;
                    e.1 += w;
                }
            }
        }
        out
    };
    let show = |label: &str, m: &BTreeMap<String, (usize, f64)>| {
        let n: usize = m.values().map(|x| x.0).sum();
        let km: f64 = m.values().map(|x| x.1).sum::<f64>() / 1000.0;
        let parts: Vec<String> = groups
            .iter()
            .map(|g| {
                let v = m.get(*g).copied().unwrap_or((0, 0.0));
                format!("{g} {} / {:.2} km", v.0, v.1 / 1000.0)
            })
            .collect();
        println!(
            "C {label:34} {n:5} samples {km:6.2} km | {}",
            parts.join(" | ")
        );
    };
    if !free {
        let c0 = count_clash(None);
        show("OSM line (before)", &c0);
        for run in &runs {
            let label = format!(
                "G={:.0}% R={:.0} slope {}{}",
                run.g * 100.0,
                run.r,
                run.slope,
                if run.pen { " + clash cost" } else { "" }
            );
            let c = count_clash(Some(&run.off));
            show(&label, &c);
        }
    }
    println!(
        "clash variant (R=10, G=36 %, 0.25 m cost per m for a clashing sample, others unshifted while choosing):"
    );
    let key = 36;
    let plain = runs
        .iter()
        .find(|r| !r.pen && r.g == 0.36 && r.r == 10.0 && r.slope == 0.25)
        .unwrap();
    let pen = runs.iter().find(|r| r.pen).unwrap();
    for grp in groups {
        let sel: Vec<usize> = tracks
            .iter()
            .copied()
            .filter(|i| lines[*i].group() == grp)
            .collect();
        let b: Vec<(f64, f64)> = sel
            .iter()
            .map(|i| (lines[*i].len, d0[&(*i, key)].0))
            .collect();
        let p: Vec<(f64, f64)> = sel.iter().map(|i| (lines[*i].len, plain.d[i].0)).collect();
        let v: Vec<(f64, f64)> = sel.iter().map(|i| (lines[*i].len, pen.d[i].0)).collect();
        println!("  {grp:17} before {}", bucket_line(&b));
        println!("  {:17} plain  {}", "", bucket_line(&p));
        println!("  {:17} clash  {}", "", bucket_line(&v));
        let mut abs_o: Vec<f64> = sel
            .iter()
            .flat_map(|i| pen.off[*i].iter().map(|o| o.abs()))
            .collect();
        println!(
            "  {:17} clash variant |offset| p50 {:.2} p90 {:.2}",
            "",
            quant(&mut abs_o, 0.5),
            quant(&mut abs_o, 0.9)
        );
    }

    if free {
        return;
    }
    // ---- D
    println!("\n#### D. three chains by hand (R=10 m, G=36 %, slope 0.25)");
    let mut picks: Vec<usize> = Vec::new();
    if let Some(i) = klinger {
        picks.push(i);
    }
    let mut cand: Vec<usize> = tracks
        .iter()
        .copied()
        .filter(|i| Some(*i) != klinger && lines[*i].len >= 100.0 && lines[*i].len <= 600.0)
        .collect();
    // the two chains that gain most in ground-only d
    let gain = |i: usize| {
        d_free(&lines[i], base, &zero[i], 0.36).0 - d_free(&lines[i], base, &plain.off[i], 0.36).0
    };
    cand.sort_by(|a, b| gain(*b).total_cmp(&gain(*a)));
    picks.extend(cand.iter().take(2));
    for i in picks {
        let l = &lines[i];
        let o = &plain.off[i];
        println!(
            "\nchain strip {} {:?} {} {:.0} m [{}]: d with pins {:.2} -> {:.2} m, ground-only d {:.2} -> {:.2} m, length {:.0} -> {:.0} m, pins {:?}/{:?}",
            l.strip,
            l.fam,
            l.group(),
            l.len,
            l.ids[0],
            d0[&(i, 36)].0,
            plain.d[&i].0,
            d_free(l, base, &zero[i], 0.36).0,
            d_free(l, base, o, 0.36).0,
            l.len,
            l.length_with(o),
            l.pin[0],
            l.pin[l.n() - 1]
        );
        println!(
            "  station | OSM ground | shifted ground | offset | side slope % (across, + = ground rises to the left) | shifted point"
        );
        let every = if l.n() > 60 { 4 } else { 2 };
        for k in (0..l.n()).step_by(every).chain(std::iter::once(l.n() - 1)) {
            let p = l.pts[k];
            let q = l.shifted(k, o[k]);
            let nn = l.nrm[k];
            let a = base.sample(p.0 + 3.0 * nn.0, p.1 + 3.0 * nn.1).height;
            let b = base.sample(p.0 - 3.0 * nn.0, p.1 - 3.0 * nn.1).height;
            println!(
                "  {:6.0} | {:7.1} | {:7.1} | {:6.2} | {:6.1} | ({:.0},{:.0})",
                l.st[k],
                l.ground(base, k, 0.0),
                l.ground(base, k, o[k]),
                o[k],
                f64::from(a - b) / 6.0 * 100.0,
                q.0,
                q.1
            );
        }
        let rough = (1..l.n())
            .map(|k| (o[k] - o[k - 1]).abs())
            .fold(0.0, f64::max);
        println!("  largest offset change between samples {rough:.2} m");
    }
    println!(
        "\n#### shift study done {:.1} s",
        t0.elapsed().as_secs_f64()
    );
}

// ======================================================================================================
// #### Junction study (2026-11-12): can every junction get one height from the ground?
//
// Throwaway measurement. Needs the strips dump and the graph dump written beside it (`<dump>.graph`, written by
// `SHIFT_DUMP`). Run `SHIFT_LOAD=<dump> JUNCTION_STUDY=1 [JUNCTION_ONLY=1]` (JUNCTION_ONLY skips the shift study).
// Junction = a graph crossing with three or more live arms, or a strip end lying on another road without a
// crossing there. See the report of the measurement for what is simplified.
// ======================================================================================================

const JN_RADII: [f64; 3] = [6.0, 10.0, 15.0];
const JN_MAIN: usize = 1;
const JN_GRID_M: f64 = 1.0;
const JN_ARM_REACH_M: f64 = 13.0;
const JN_ROUNDS: usize = 50;
const JN_MARGIN_M: f64 = 0.001;
const JN_TOL_M: f64 = 0.01;
/// Ratio of the mobility of two junctions whose most important roads differ by one rank.
const JN_RANK_RATIO: f64 = 2.0;

#[derive(Clone, Copy)]
struct Ceils {
    asf: f64,
    grav: f64,
    mud: f64,
}
const CEIL_PROPOSED: Ceils = Ceils {
    asf: 0.27,
    grav: 0.36,
    mud: 0.45,
};
const CEIL_TODAY: Ceils = Ceils {
    asf: 0.27,
    grav: 0.27,
    mud: 0.252,
};
fn ceil_of(f: Family, c: &Ceils) -> f64 {
    match f {
        Family::Grav => c.grav,
        Family::Mud => c.mud,
        _ => c.asf,
    }
}
fn fam_rank(f: Family) -> usize {
    match f {
        Family::Asf1 => 0,
        Family::Asf2 => 1,
        Family::Asf3 => 2,
        Family::City => 3,
        Family::Grav => 4,
        Family::Mud => 5,
    }
}

struct GraphDump {
    cross: Vec<((f64, f64), Vec<String>)>,
    joins: Vec<(f64, f64)>,
}

fn load_graph(path: &str) -> Option<GraphDump> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut g = GraphDump {
        cross: Vec::new(),
        joins: Vec::new(),
    };
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] == "C" {
            let arms = f[4]
                .split(',')
                .filter(|a| !a.is_empty())
                .map(|a| a.split('@').next().unwrap().to_string())
                .collect();
            g.cross
                .push(((f[2].parse().unwrap(), f[3].parse().unwrap()), arms));
        } else if f[0] == "P" {
            g.joins.push((f[1].parse().unwrap(), f[2].parse().unwrap()));
        }
    }
    Some(g)
}

#[derive(Clone)]
struct JArm {
    line: usize,
    station: f64,
    end: Option<usize>,
    half: f64,
}

struct Junc {
    pt: (f64, f64),
    kind: &'static str,
    graph_arms: usize,
    arms: Vec<JArm>,
    med: [f64; 3],
    mean: [f64; 3],
    spread: [f64; 3],
    point_h: f64,
    rank: usize,
}

/// Nearest point of a line to `p`: distance and station.
fn jn_project(line: &Line, p: (f64, f64)) -> (f64, f64) {
    let mut best = (f64::INFINITY, 0.0);
    for k in 0..line.jp.len() - 1 {
        let (d, t) = seg_dist(p, line.jp[k], line.jp[k + 1]);
        if d < best.0 {
            best = (d, line.cs[k] + t * (line.cs[k + 1] - line.cs[k]));
        }
    }
    best
}

/// Half the length of the junction piece of a strip that carries a station, or 0.
fn jn_half(strip: &DStrip, line: &Line, station: f64) -> f64 {
    for k in 0..line.jp.len() - 1 {
        if strip.junction.get(k).copied().unwrap_or(false) {
            let mid = 0.5 * (line.cs[k] + line.cs[k + 1]);
            if (mid - station).abs() < 4.0 {
                return 0.5 * (line.cs[k + 1] - line.cs[k]);
            }
        }
    }
    0.0
}

fn jn_end(line: &Line, station: f64) -> Option<usize> {
    if station <= 3.0 {
        Some(0)
    } else if station >= line.len - 3.0 {
        Some(1)
    } else {
        None
    }
}

/// Planned height of a strip at a station.
fn jn_z(strip: &DStrip, line: &Line, s: f64) -> f64 {
    let nseg = line.jp.len() - 1;
    let seg = (line.cs.partition_point(|c| *c <= s).saturating_sub(1)).min(nseg - 1);
    let l = line.cs[seg + 1] - line.cs[seg];
    let t = if l > 0.0 {
        ((s - line.cs[seg]) / l).clamp(0.0, 1.0)
    } else {
        0.0
    };
    strip.joints[seg].2 + t * (strip.joints[seg + 1].2 - strip.joints[seg].2)
}

fn jn_med(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

fn jn_ground(base: &Heightmap, pt: (f64, f64)) -> ([f64; 3], [f64; 3], [f64; 3], f64) {
    let rmax = JN_RADII[2];
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let n = (rmax / JN_GRID_M) as i32;
    for i in -n..=n {
        for j in -n..=n {
            let (dx, dy) = (f64::from(i) * JN_GRID_M, f64::from(j) * JN_GRID_M);
            pts.push((
                dx.hypot(dy),
                f64::from(base.sample(pt.0 + dx, pt.1 + dy).height),
            ));
        }
    }
    let (mut med, mut mean, mut spread) = ([0.0; 3], [0.0; 3], [0.0; 3]);
    for (k, r) in JN_RADII.iter().enumerate() {
        let mut v: Vec<f64> = pts.iter().filter(|p| p.0 <= *r).map(|p| p.1).collect();
        mean[k] = v.iter().sum::<f64>() / v.len() as f64;
        let (lo, hi) = v
            .iter()
            .fold((f64::MAX, f64::MIN), |a, x| (a.0.min(*x), a.1.max(*x)));
        spread[k] = hi - lo;
        med[k] = jn_med(&mut v);
    }
    (med, mean, spread, f64::from(base.sample(pt.0, pt.1).height))
}

struct Link {
    line: usize,
    a: Option<usize>,
    b: Option<usize>,
    from: f64,
    to: f64,
    ds: f64,
    len: f64,
    fam: Family,
    group: String,
    g: Vec<f64>,
    zp: Vec<f64>,
    br: Vec<bool>,
}

impl Link {
    fn dl(&self) -> f64 {
        self.len / (self.g.len() - 1) as f64
    }
    fn jj(&self) -> bool {
        self.a.is_some() && self.b.is_some()
    }
}

fn jn_build(
    strips: &[DStrip],
    lines: &[Line],
    graph: &GraphDump,
    base: &Heightmap,
) -> (Vec<Junc>, Vec<Link>) {
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (li, l) in lines.iter().enumerate() {
        for id in &l.ids {
            for part in id.split('|') {
                let e = by_id.entry(part.to_string()).or_default();
                if !e.contains(&li) {
                    e.push(li);
                }
            }
        }
    }
    let strip_line: HashMap<usize, usize> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| (l.strip, i))
        .collect();
    let mut juncs: Vec<Junc> = Vec::new();
    let mut claimed: BTreeSet<(usize, usize)> = BTreeSet::new();
    let (mut dangling, mut few_arms) = (0, 0);
    for (pt, arms) in &graph.cross {
        if arms.len() < 3 {
            continue;
        }
        let mut cand: BTreeSet<usize> = BTreeSet::new();
        for a in arms {
            if let Some(v) = by_id.get(a) {
                cand.extend(v.iter().copied());
            }
        }
        let mut ja: Vec<JArm> = Vec::new();
        for li in cand {
            let (d, s) = jn_project(&lines[li], *pt);
            if d <= JN_ARM_REACH_M {
                let half = jn_half(&strips[lines[li].strip], &lines[li], s);
                let end = jn_end(&lines[li], s);
                ja.push(JArm {
                    line: li,
                    station: match end {
                        Some(0) => 0.0,
                        Some(_) => lines[li].len,
                        None => s,
                    },
                    end,
                    half: if end.is_some() { 0.0 } else { half },
                });
            }
        }
        if ja.is_empty() {
            dangling += 1;
            continue;
        }
        let distinct: BTreeSet<usize> = ja.iter().map(|a| a.line).collect();
        if distinct.len() < 2 {
            few_arms += 1;
            continue;
        }
        let kind = if ja.iter().any(|a| a.half > 0.0) {
            "piece"
        } else if graph
            .joins
            .iter()
            .any(|p| (p.0 - pt.0).hypot(p.1 - pt.1) < 1.5)
        {
            "join"
        } else {
            "other"
        };
        for a in &ja {
            if let Some(e) = a.end {
                claimed.insert((a.line, e));
            }
        }
        juncs.push(Junc {
            pt: *pt,
            kind,
            graph_arms: arms.len(),
            arms: ja,
            med: [0.0; 3],
            mean: [0.0; 3],
            spread: [0.0; 3],
            point_h: 0.0,
            rank: 0,
        });
    }
    let n_graph = juncs.len();
    println!(
        "graph crossings with 3+ arms: {} ; kept as junctions {} ; no strip within {} m {} ; dropped because fewer than 2 strips meet there {}",
        graph.cross.iter().filter(|c| c.1.len() >= 3).count(),
        n_graph,
        JN_ARM_REACH_M,
        dangling,
        few_arms
    );
    // strip ends lying on another road without a crossing
    let mut near_graph = 0;
    let mut owner_missing = 0;
    for li in 0..lines.len() {
        for e in 0..2 {
            if claimed.contains(&(li, e)) {
                continue;
            }
            let l = &lines[li];
            let p = if e == 0 {
                l.jp[0]
            } else {
                l.jp[l.jp.len() - 1]
            };
            let Some((owner, _, _)) = meeting_d(strips, l.strip, p) else {
                continue;
            };
            let Some(&ol) = strip_line.get(&owner) else {
                owner_missing += 1;
                continue;
            };
            if let Some(bi) = juncs[..n_graph]
                .iter()
                .enumerate()
                .map(|(i, j)| (i, (j.pt.0 - p.0).hypot(j.pt.1 - p.1)))
                .filter(|x| x.1 < 12.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|x| x.0)
            {
                near_graph += 1;
                juncs[bi].arms.push(JArm {
                    line: li,
                    station: if e == 0 { 0.0 } else { l.len },
                    end: Some(e),
                    half: 0.0,
                });
                continue;
            }
            let (_, os) = jn_project(&lines[ol], p);
            let my = JArm {
                line: li,
                station: if e == 0 { 0.0 } else { l.len },
                end: Some(e),
                half: 0.0,
            };
            let existing = juncs
                .iter()
                .skip(n_graph)
                .position(|j| (j.pt.0 - p.0).hypot(j.pt.1 - p.1) < 6.0)
                .map(|i| i + n_graph);
            match existing {
                Some(ji) => {
                    juncs[ji].arms.push(my);
                    if !juncs[ji].arms.iter().any(|a| a.line == ol) {
                        juncs[ji].arms.push(JArm {
                            line: ol,
                            station: os,
                            end: jn_end(&lines[ol], os),
                            half: 0.0,
                        });
                    }
                }
                None => juncs.push(Junc {
                    pt: p,
                    kind: "end on road",
                    graph_arms: 0,
                    arms: vec![
                        my,
                        JArm {
                            line: ol,
                            station: os,
                            end: jn_end(&lines[ol], os),
                            half: 0.0,
                        },
                    ],
                    med: [0.0; 3],
                    mean: [0.0; 3],
                    spread: [0.0; 3],
                    point_h: 0.0,
                    rank: 0,
                }),
            }
        }
    }
    println!(
        "strip ends on another road without a crossing: junctions {} (ends attached to a crossing within 12 m that does not list them: {near_graph}; owner strip missing: {owner_missing})",
        juncs.len() - n_graph
    );
    {
        let at_arm: BTreeSet<(usize, usize)> = juncs
            .iter()
            .flat_map(|j| j.arms.iter().filter_map(|a| a.end.map(|e| (a.line, e))))
            .collect();
        let (mut pinned, mut covered) = (0, 0);
        for (li, l) in lines.iter().enumerate() {
            for e in 0..2 {
                let p = if e == 0 {
                    l.jp[0]
                } else {
                    l.jp[l.jp.len() - 1]
                };
                if meeting_d(strips, l.strip, p).is_some() {
                    pinned += 1;
                    if at_arm.contains(&(li, e)) {
                        covered += 1;
                    }
                }
            }
        }
        println!(
            "strip ends pinned to an earlier strip today: {pinned}; of these at a junction arm here: {covered}"
        );
    }
    for j in &mut juncs {
        let (m, me, sp, ph) = jn_ground(base, j.pt);
        j.med = m;
        j.mean = me;
        j.spread = sp;
        j.point_h = ph;
        j.rank = j
            .arms
            .iter()
            .map(|a| fam_rank(lines[a.line].fam))
            .min()
            .unwrap_or(5);
    }
    // links
    let mut per_line: Vec<Vec<(f64, Option<usize>, f64)>> = vec![Vec::new(); lines.len()];
    for (ji, j) in juncs.iter().enumerate() {
        for a in &j.arms {
            per_line[a.line].push((a.station, Some(ji), a.half));
        }
    }
    let mut links: Vec<Link> = Vec::new();
    let mut isolated = 0;
    for (li, entries) in per_line.iter_mut().enumerate() {
        let l = &lines[li];
        if entries.is_empty() {
            isolated += 1;
            continue;
        }
        entries.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut e: Vec<(f64, Option<usize>, f64)> = Vec::new();
        for x in entries.iter() {
            if e.last().is_some_and(|y| y.1 == x.1) {
                continue;
            }
            e.push(*x);
        }
        if e[0].0 > 0.5 {
            e.insert(0, (0.0, None, 0.0));
        }
        if e[e.len() - 1].0 < l.len - 0.5 {
            e.push((l.len, None, 0.0));
        }
        for w in e.windows(2) {
            let (a, b) = (w[0], w[1]);
            let from = a.0 + a.2;
            let to = b.0 - b.2;
            let len = (to - from).max(1.0);
            let n = ((len / SAMPLE_STEP_M).round() as usize).max(1);
            let mut g = Vec::new();
            let mut zp = Vec::new();
            let mut br = Vec::new();
            for k in 0..=n {
                let sta = (from + len * k as f64 / n as f64).clamp(0.0, l.len);
                let (p, seg) = l.at_station(sta);
                g.push(f64::from(base.sample(p.0, p.1).height));
                zp.push(jn_z(&strips[l.strip], l, sta));
                br.push(
                    k > 0
                        && k < n
                        && l.jbridge[seg]
                        && sta > l.cs[seg] + 0.01
                        && sta < l.cs[seg + 1] - 0.01,
                );
            }
            // ground under a deck: the line between the abutments
            let mut i = 0;
            while i <= n {
                if br[i] {
                    let s0 = i;
                    while i <= n && br[i] {
                        i += 1;
                    }
                    let (left, right) = (s0 - 1, i.min(n));
                    for k in s0..i {
                        let f = (k - left) as f64 / (right - left) as f64;
                        g[k] = g[left] + f * (g[right] - g[left]);
                    }
                } else {
                    i += 1;
                }
            }
            links.push(Link {
                line: li,
                a: a.1,
                b: b.1,
                from,
                to,
                ds: b.0 - a.0,
                len,
                fam: l.fam,
                group: l.group(),
                g,
                zp,
                br,
            });
        }
    }
    println!("strips without any junction: {isolated}");
    (juncs, links)
}

fn jn_d(l: &Link, grade: f64, pa: Option<f64>, pb: Option<f64>) -> Option<f64> {
    let n = l.g.len();
    let dl = vec![l.dl(); n - 1];
    let mut pin = vec![None; n];
    pin[0] = pa;
    pin[n - 1] = pb;
    if fails_at(&l.g, &dl, &pin, grade, 400.0).is_some() {
        return None;
    }
    if fails_at(&l.g, &dl, &pin, grade, 0.0).is_none() {
        return Some(0.0);
    }
    let (mut lo, mut hi) = (0.0_f64, 400.0_f64);
    for _ in 0..34 {
        let mid = 0.5 * (lo + hi);
        if fails_at(&l.g, &dl, &pin, grade, mid).is_some() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(hi)
}

fn jn_line(label: &str, mut v: Vec<f64>, thr: &[f64]) {
    let n = v.len();
    let over: Vec<String> = thr
        .iter()
        .map(|t| format!(">{t}: {}", v.iter().filter(|x| **x > *t).count()))
        .collect();
    println!(
        "  {label:42} n {n:5} p50 {:7.2} p90 {:7.2} max {:7.2} | {}",
        quant(&mut v, 0.5),
        quant(&mut v, 0.9),
        quant(&mut v, 1.0),
        over.join(", ")
    );
}

fn jn_group_rows(title: &str, items: &[(String, f64, f64)]) {
    // items: (group, len, d)
    println!("  {title}");
    let mut groups: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    for (g, l, d) in items {
        groups.entry(g.clone()).or_default().push((*l, *d));
    }
    for (g, v) in &groups {
        println!("    {g:17} {}", bucket_line(v).replace(" ch ", " lk "));
    }
    let all: Vec<(f64, f64)> = items.iter().map(|x| (x.1, x.2)).collect();
    println!(
        "    {:17} {}",
        "all",
        bucket_line(&all).replace(" ch ", " lk ")
    );
}

fn jn_over_stats(label: &str, links: &[&Link], h: &[f64], c: &Ceils) {
    // per group: n, km, over share, excess distribution
    let mut groups: BTreeMap<String, Vec<(f64, f64, f64, f64)>> = BTreeMap::new();
    for l in links {
        let (a, b) = (l.a.unwrap(), l.b.unwrap());
        let dh = (h[a] - h[b]).abs();
        let cl = ceil_of(l.fam, c);
        let need = dh / l.len;
        groups.entry(l.group.clone()).or_default().push((
            l.len,
            need,
            (dh - cl * l.len).max(0.0),
            (need - cl).max(0.0),
        ));
    }
    println!("  {label}");
    let all: Vec<(f64, f64, f64, f64)> = groups.values().flatten().copied().collect();
    let mut names: Vec<String> = groups.keys().cloned().collect();
    names.push("all".to_string());
    for g in &names {
        let v: &Vec<(f64, f64, f64, f64)> = if g == "all" { &all } else { &groups[g] };
        let n = v.len();
        let km: f64 = v.iter().map(|x| x.0).sum::<f64>() / 1000.0;
        let over: Vec<&(f64, f64, f64, f64)> = v.iter().filter(|x| x.2 > 1e-9).collect();
        let okm: f64 = 0.0 + over.iter().map(|x| x.0).sum::<f64>() / 1000.0;
        let mut ex: Vec<f64> = over.iter().map(|x| x.2).collect();
        let mut eg: Vec<f64> = over.iter().map(|x| x.3 * 100.0).collect();
        println!(
            "    {g:17} {n:5} lk {km:6.2} km | over {:4} lk ({:4.1} %) {okm:5.2} km ({:4.1} %) | move toward each other m: p50 {:5.2} p90 {:5.2} max {:6.2}; >1 m {}, >2 m {}, >5 m {} | excess grade pp: p50 {:5.1} p90 {:5.1} max {:6.1}",
            over.len(),
            over.len() as f64 / n.max(1) as f64 * 100.0,
            okm / km.max(1e-9) * 100.0,
            quant(&mut ex, 0.5),
            quant(&mut ex, 0.9),
            quant(&mut ex, 1.0),
            ex.iter().filter(|x| **x > 1.0).count(),
            ex.iter().filter(|x| **x > 2.0).count(),
            ex.iter().filter(|x| **x > 5.0).count(),
            quant(&mut eg, 0.5),
            quant(&mut eg, 0.9),
            quant(&mut eg, 1.0),
        );
    }
    // by length
    let buckets: [(&str, f64, f64); 4] = [
        ("< 15 m", 0.0, 15.0),
        ("15-30 m", 15.0, 30.0),
        ("30-60 m", 30.0, 60.0),
        (">= 60 m", 60.0, f64::MAX),
    ];
    for (name, lo, hi) in buckets {
        let in_b: Vec<&(f64, f64, f64, f64)> =
            all.iter().filter(|x| x.0 >= lo && x.0 < hi).collect();
        let over: Vec<&&(f64, f64, f64, f64)> = in_b.iter().filter(|x| x.2 > 1e-9).collect();
        let mut ex: Vec<f64> = over.iter().map(|x| x.2).collect();
        println!(
            "    length {name:8} links {:5} of which over {:4} ({:4.1} %) | move m p50 {:5.2} p90 {:5.2} max {:6.2}",
            in_b.len(),
            over.len(),
            over.len() as f64 / in_b.len().max(1) as f64 * 100.0,
            quant(&mut ex, 0.5),
            quant(&mut ex, 0.9),
            quant(&mut ex, 1.0)
        );
    }
}

fn jn_relax(h0: &[f64], cons: &[(usize, usize, f64)], mob: &[f64], rounds: usize) -> Vec<f64> {
    let mut h = h0.to_vec();
    for _ in 0..rounds {
        for &(a, b, cl) in cons {
            let dh = h[b] - h[a];
            let ex = dh.abs() - cl;
            if ex > 0.0 {
                let x = ex + JN_MARGIN_M;
                let sa = mob[a] / (mob[a] + mob[b]);
                let sgn = dh.signum();
                h[a] += sgn * x * sa;
                h[b] -= sgn * x * (1.0 - sa);
            }
        }
    }
    h
}

fn jn_over(h: &[f64], cons: &[(usize, usize, f64)]) -> Vec<usize> {
    cons.iter()
        .enumerate()
        .filter(|(_, c)| (h[c.1] - h[c.0]).abs() - c.2 > JN_TOL_M)
        .map(|(i, _)| i)
        .collect()
}

/// Smallest delta such that every junction of the component can lie within delta of its median and all
/// constraints hold; interval propagation, exact.
fn jn_minimax(nodes: &[usize], cons: &[(usize, usize, f64)], med: &[f64]) -> f64 {
    let feasible = |delta: f64| -> bool {
        let mut lo: HashMap<usize, f64> = nodes.iter().map(|n| (*n, med[*n] - delta)).collect();
        let mut hi: HashMap<usize, f64> = nodes.iter().map(|n| (*n, med[*n] + delta)).collect();
        for _ in 0..nodes.len() + 3 {
            let mut changed = false;
            for &(a, b, w) in cons {
                let (la, lb, ha, hb) = (lo[&a], lo[&b], hi[&a], hi[&b]);
                if la - w > lb {
                    lo.insert(b, la - w);
                    changed = true;
                }
                if lb - w > la {
                    lo.insert(a, lb - w);
                    changed = true;
                }
                if ha + w < hb {
                    hi.insert(b, ha + w);
                    changed = true;
                }
                if hb + w < ha {
                    hi.insert(a, hb + w);
                    changed = true;
                }
            }
            if nodes.iter().any(|n| lo[n] > hi[n] + 1e-9) {
                return false;
            }
            if !changed {
                return true;
            }
        }
        false
    };
    if feasible(0.0) {
        return 0.0;
    }
    let (mut lo, mut hi) = (0.0_f64, 200.0_f64);
    for _ in 0..30 {
        let mid = 0.5 * (lo + hi);
        if feasible(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

fn jn_components(nj: usize, cons: &[(usize, usize, f64)]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..nj).collect();
    fn find(p: &mut [usize], x: usize) -> usize {
        let mut r = x;
        while p[r] != r {
            r = p[r];
        }
        let mut c = x;
        while p[c] != r {
            let nx = p[c];
            p[c] = r;
            c = nx;
        }
        r
    }
    for &(a, b, _) in cons {
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    let mut comp: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let used: BTreeSet<usize> = cons.iter().flat_map(|c| [c.0, c.1]).collect();
    for j in used {
        let r = find(&mut parent, j);
        comp.entry(r).or_default().push(j);
    }
    comp.into_values().collect()
}

fn junction_study(strips: &[DStrip], base: &Heightmap, graph: &GraphDump) {
    let t0 = Instant::now();
    let lines = build_lines(strips, base);
    println!(
        "\n#### junction study: size {}, cell {} m, {} strips, {} lines",
        base.size,
        base.cell_m,
        strips.len(),
        lines.len()
    );
    let (juncs, links) = jn_build(strips, &lines, graph, base);
    let nj = juncs.len();
    let fam_of = |a: &JArm| lines[a.line].fam;
    // ---------------- A
    println!("\n#### junction study A. the junctions");
    for kind in ["piece", "join", "other", "end on road"] {
        let v: Vec<&Junc> = juncs.iter().filter(|j| j.kind == kind).collect();
        println!("  kind {kind:12} {:5}", v.len());
    }
    println!("  total {nj}");
    let mut by_arms: BTreeMap<(String, usize), usize> = BTreeMap::new();
    for j in &juncs {
        let na = if j.graph_arms > 0 { j.graph_arms } else { 3 };
        *by_arms.entry((j.kind.to_string(), na.min(6))).or_default() += 1;
    }
    println!(
        "  by kind and number of arms (graph stretch ends, 'end on road' counted as 3; 6 = 6 or more): {by_arms:?}"
    );
    let mut combos: BTreeMap<String, usize> = BTreeMap::new();
    let mut most_important: BTreeMap<String, usize> = BTreeMap::new();
    for j in &juncs {
        let mut f: Vec<Family> = j.arms.iter().map(fam_of).collect();
        f.sort();
        f.dedup();
        *combos
            .entry(
                f.iter()
                    .map(|x| format!("{x:?}"))
                    .collect::<Vec<_>>()
                    .join("+"),
            )
            .or_default() += 1;
        *most_important.entry(format!("{:?}", f[0])).or_default() += 1;
    }
    let mut cv: Vec<(String, usize)> = combos.into_iter().collect();
    cv.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("  families meeting (distinct families of the strips found at the junction), top 14:");
    for (k, v) in cv.iter().take(14) {
        println!("    {k:28} {v}");
    }
    println!("  by most important family at the junction: {most_important:?}");
    let jj: Vec<&Link> = links.iter().filter(|l| l.jj() && l.a != l.b).collect();
    let fe: Vec<&Link> = links.iter().filter(|l| !l.jj()).collect();
    let share = |v: &[f64], t: f64| {
        v.iter().filter(|x| **x < t).count() as f64 / v.len().max(1) as f64 * 100.0
    };
    for (label, f) in [
        ("centre to centre along the road (ds)", 0),
        ("edge to edge (length L used for the grade)", 1),
    ] {
        let mut v: Vec<f64> = jj
            .iter()
            .map(|l| if f == 0 { l.ds } else { l.len })
            .collect();
        println!(
            "  junction-to-junction links: {} ({:.1} km) {label}: median {:.1} m, p10 {:.1} m | share below 12.5 m {:.1} %, 25 m {:.1} %, 50 m {:.1} %",
            v.len(),
            jj.iter().map(|l| l.len).sum::<f64>() / 1000.0,
            quant(&mut v.clone(), 0.5),
            quant(&mut v, 0.1),
            share(&v, 12.5),
            share(&v, 25.0),
            share(&v, 50.0)
        );
    }
    let same = links.iter().filter(|l| l.a.is_some() && l.a == l.b).count();
    println!("  links from a junction to itself (dropped): {same}");
    let mut nn: Vec<f64> = Vec::new();
    for (i, a) in juncs.iter().enumerate() {
        let d = juncs
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != i)
            .map(|(_, b)| (a.pt.0 - b.pt.0).hypot(a.pt.1 - b.pt.1))
            .fold(f64::MAX, f64::min);
        nn.push(d);
    }
    println!(
        "  straight distance to the nearest other junction: median {:.1} m, p10 {:.1} m | share below 5 m {:.1} %, 12.5 m {:.1} %, 25 m {:.1} %, 50 m {:.1} %",
        quant(&mut nn.clone(), 0.5),
        quant(&mut nn.clone(), 0.1),
        share(&nn, 5.0),
        share(&nn, 12.5),
        share(&nn, 25.0),
        share(&nn, 50.0)
    );
    let mut lens: Vec<f64> = jj.iter().map(|l| l.len).collect();
    println!(
        "  links of length (edge to edge) clamped to 1 m (junction pieces touch): {}",
        lens.iter().filter(|x| **x <= 1.0 + 1e-9).count()
    );
    let _ = quant(&mut lens, 0.5);
    println!(
        "  links to a free end (dead end, map edge, strip end that is not a junction): {} ({:.1} km)",
        fe.len(),
        fe.iter().map(|l| l.len).sum::<f64>() / 1000.0
    );
    // ---------------- B
    println!(
        "\n#### junction study B. the height from the ground (grid {JN_GRID_M} m inside a circle; radii {JN_RADII:?} m; main choice R = {} m)",
        JN_RADII[JN_MAIN]
    );
    let col = |f: &dyn Fn(&Junc) -> f64| -> Vec<f64> { juncs.iter().map(f).collect() };
    println!("  median against mean:");
    for k in 0..3 {
        jn_line(
            &format!("|median - mean| R={} m", JN_RADII[k]),
            col(&|j| (j.med[k] - j.mean[k]).abs()),
            &[0.25, 0.5, 1.0],
        );
    }
    println!("  between radii (median):");
    for (a, b) in [(0, 1), (1, 2), (0, 2)] {
        jn_line(
            &format!("|med R={} - med R={}|", JN_RADII[a], JN_RADII[b]),
            col(&|j| (j.med[a] - j.med[b]).abs()),
            &[0.25, 0.5, 1.0],
        );
    }
    println!("  median against the single height at the junction point:");
    for k in 0..3 {
        jn_line(
            &format!("|median R={} m - point|", JN_RADII[k]),
            col(&|j| (j.med[k] - j.point_h).abs()),
            &[0.25, 0.5, 1.0],
        );
    }
    println!("  spread of the ground in the footprint (max - min), the terrain to cut and fill:");
    for k in 0..3 {
        jn_line(
            &format!("spread R={} m", JN_RADII[k]),
            col(&|j| j.spread[k]),
            &[2.0, 5.0, 10.0],
        );
    }
    {
        let mut v: Vec<f64> = Vec::new();
        for l in &links {
            if let Some(a) = l.a {
                v.push((l.g[0] - juncs[a].med[JN_MAIN]).abs());
            }
            if let Some(b) = l.b {
                v.push((l.g[l.g.len() - 1] - juncs[b].med[JN_MAIN]).abs());
            }
        }
        println!(
            "  the ground where the road arm leaves the junction footprint against the junction's median R={} m (cut or fill at the arm end if the junction is levelled to the median):",
            JN_RADII[JN_MAIN]
        );
        jn_line("|ground at the link end - median|", v, &[1.0, 2.0, 5.0]);
    }
    // today's planned heights
    let mut diff_today: Vec<f64> = Vec::new();
    let mut arm_spread: Vec<f64> = Vec::new();
    let mut diff_by_kind: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for j in &juncs {
        let mut zs: Vec<(usize, f64)> = j
            .arms
            .iter()
            .map(|a| {
                let l = &lines[a.line];
                (fam_rank(l.fam), jn_z(&strips[l.strip], l, a.station))
            })
            .collect();
        let best = zs.iter().map(|z| z.0).min().unwrap_or(5);
        let zm = zs
            .iter()
            .filter(|z| z.0 == best)
            .map(|z| z.1)
            .next()
            .unwrap_or(0.0);
        let d = (zm - j.med[JN_MAIN]).abs();
        diff_today.push(d);
        diff_by_kind.entry(j.kind).or_default().push(d);
        zs.sort_by(|a, b| a.1.total_cmp(&b.1));
        arm_spread.push(zs[zs.len() - 1].1 - zs[0].1);
    }
    println!(
        "  today's planned height of the most important road at the junction against the ground median R={} m:",
        JN_RADII[JN_MAIN]
    );
    jn_line("|planned - median|", diff_today, &[2.0, 5.0, 10.0]);
    for (k, v) in diff_by_kind {
        jn_line(&format!("  of which {k}"), v, &[2.0, 5.0, 10.0]);
    }
    jn_line(
        "today: planned heights of the arms disagree",
        arm_spread,
        &[0.5, 2.0, 5.0],
    );
    // ---------------- C
    let h0: Vec<f64> = juncs.iter().map(|j| j.med[JN_MAIN]).collect();
    println!(
        "\n#### junction study C. can neighbouring junctions be connected at their ground medians?"
    );
    println!(
        "  (length L = road between the junction edges: a through arm with a junction piece loses half the piece's length (level), a side arm starts at its port, a join without a piece loses nothing; L at least 1 m; grade = |median difference| / L)"
    );
    for (name, c) in [
        (
            "PROPOSED ceilings Asf/City 27 %, Grav 36 %, Mud 45 %",
            &CEIL_PROPOSED,
        ),
        (
            "TODAY's ceilings Asf/City 27 %, Grav 27 %, Mud 25.2 %",
            &CEIL_TODAY,
        ),
    ] {
        jn_over_stats(name, &jj, &h0, c);
    }
    // sensitivity: centre to centre length
    {
        let mut n_over = 0;
        for l in &jj {
            let dh = (h0[l.a.unwrap()] - h0[l.b.unwrap()]).abs();
            if dh > ceil_of(l.fam, &CEIL_PROPOSED) * l.ds.max(1.0) {
                n_over += 1;
            }
        }
        println!(
            "  sensitivity (proposed ceilings): with the length centre to centre (no level piece taken out) {n_over} of {} links are over",
            jj.len()
        );
    }
    // free-end links
    println!(
        "  links between a junction and a free end (cannot be infeasible), d with the junction end on its median and the other end free, proposed ceilings:"
    );
    {
        let mut items: Vec<(String, f64, f64)> = Vec::new();
        for l in &fe {
            let c = ceil_of(l.fam, &CEIL_PROPOSED);
            let (pa, pb) = match (l.a, l.b) {
                (Some(a), None) => (Some(h0[a]), None),
                (None, Some(b)) => (None, Some(h0[b])),
                _ => continue,
            };
            if let Some(d) = jn_d(l, c, pa, pb) {
                items.push((l.group.clone(), l.len, d));
            }
        }
        jn_group_rows("free-end links, one end at the junction median", &items);
        let items0: Vec<(String, f64, f64)> = fe
            .iter()
            .filter_map(|l| {
                jn_d(l, ceil_of(l.fam, &CEIL_PROPOSED), None, None)
                    .map(|d| (l.group.clone(), l.len, d))
            })
            .collect();
        jn_group_rows("the same links with both ends free (floor)", &items0);
    }
    // d tables for links that pass the test
    let passing: Vec<&Link> = jj
        .iter()
        .copied()
        .filter(|l| {
            (h0[l.a.unwrap()] - h0[l.b.unwrap()]).abs()
                <= ceil_of(l.fam, &CEIL_PROPOSED) * l.len + 1e-9
        })
        .collect();
    println!(
        "  links that pass the mean-grade test (proposed ceilings): {} of {} ({:.1} km of {:.1} km)",
        passing.len(),
        jj.len(),
        passing.iter().map(|l| l.len).sum::<f64>() / 1000.0,
        jj.iter().map(|l| l.len).sum::<f64>() / 1000.0
    );
    let d_report = |title: &str, set: &[&Link], h: &[f64], c: &Ceils| {
        let mut median_ends: Vec<(String, f64, f64)> = Vec::new();
        let mut today_pins: Vec<(String, f64, f64)> = Vec::new();
        let mut free: Vec<(String, f64, f64)> = Vec::new();
        let mut today_actual: Vec<(String, f64, f64)> = Vec::new();
        let mut none_me = 0;
        for l in set {
            let g = ceil_of(l.fam, c);
            let n = l.g.len();
            match jn_d(l, g, Some(h[l.a.unwrap()]), Some(h[l.b.unwrap()])) {
                Some(d) => median_ends.push((l.group.clone(), l.len, d)),
                None => none_me += 1,
            }
            if let Some(d) = jn_d(l, g, Some(l.zp[0]), Some(l.zp[n - 1])) {
                today_pins.push((l.group.clone(), l.len, d));
            }
            if let Some(d) = jn_d(l, g, None, None) {
                free.push((l.group.clone(), l.len, d));
            }
            let act = (1..n.saturating_sub(1))
                .filter(|k| !l.br[*k])
                .map(|k| (l.zp[k] - l.g[k]).abs())
                .fold(0.0, f64::max);
            today_actual.push((l.group.clone(), l.len, act));
        }
        println!(
            "\n  {title} ({} links; ends at the junction heights not joinable: {none_me})",
            set.len()
        );
        jn_group_rows("d with both ends on the junction heights", &median_ends);
        jn_group_rows(
            "same links, ends at today's planned heights of that road (min d, same ceiling)",
            &today_pins,
        );
        jn_group_rows(
            "same links, today's planned profile: largest |planned - ground| (not a minimum)",
            &today_actual,
        );
        jn_group_rows("same links, both ends free (floor)", &free);
    };
    d_report(
        "d for links that pass the test, proposed ceilings, junctions at the ground median",
        &passing,
        &h0,
        &CEIL_PROPOSED,
    );
    // ---------------- D
    println!(
        "\n#### junction study D. the repair rule: {JN_ROUNDS} rounds, links in a fixed order, move = excess + {JN_MARGIN_M} m shared by mobility; importance: mobility {JN_RANK_RATIO}^rank (Asf1 rank 0 ... Mud rank 5, most important road at the junction)"
    );
    let mut repaired: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for (cname, c) in [("PROPOSED", &CEIL_PROPOSED), ("TODAY", &CEIL_TODAY)] {
        let cons: Vec<(usize, usize, f64)> = jj
            .iter()
            .map(|l| (l.a.unwrap(), l.b.unwrap(), ceil_of(l.fam, c) * l.len))
            .collect();
        let over0 = jn_over(&h0, &cons).len();
        println!(
            "\n  ceilings {cname}: {} links, {over0} over before the repair",
            cons.len()
        );
        let comps = jn_components(nj, &cons);
        let mut mm: Vec<(f64, usize, usize)> = Vec::new();
        for comp in &comps {
            let set: BTreeSet<usize> = comp.iter().copied().collect();
            let cc: Vec<(usize, usize, f64)> = cons
                .iter()
                .copied()
                .filter(|c| set.contains(&c.0))
                .collect();
            mm.push((jn_minimax(comp, &cc, &h0), comp.len(), cc.len()));
        }
        let mut dstar: Vec<f64> = mm.iter().map(|x| x.0).collect();
        println!(
            "  reference, whole-network minimax per connected component of the junction graph (smallest possible largest move): {} components, {} need any move; delta* max {:.2} m, p90 {:.2} m; components with delta* > 1 m {}, > 2 m {}, > 5 m {}",
            comps.len(),
            mm.iter().filter(|x| x.0 > 1e-6).count(),
            quant(&mut dstar, 1.0),
            quant(&mut dstar, 0.9),
            mm.iter().filter(|x| x.0 > 1.0).count(),
            mm.iter().filter(|x| x.0 > 2.0).count(),
            mm.iter().filter(|x| x.0 > 5.0).count()
        );
        let biggest = mm.iter().map(|x| x.1).max().unwrap_or(0);
        println!("  largest component: {biggest} junctions");
        for (wname, mob) in [
            ("equal weights", vec![1.0; nj]),
            (
                "by importance",
                juncs
                    .iter()
                    .map(|j| JN_RANK_RATIO.powi(j.rank as i32))
                    .collect(),
            ),
        ] {
            for rounds in [JN_ROUNDS, 500] {
                let h = jn_relax(&h0, &cons, &mob, rounds);
                let still = jn_over(&h, &cons);
                let mut resid: Vec<f64> = still
                    .iter()
                    .map(|i| (h[cons[*i].1] - h[cons[*i].0]).abs() - cons[*i].2)
                    .collect();
                let mv: Vec<f64> = (0..nj).map(|i| (h[i] - h0[i]).abs()).collect();
                let moved: Vec<f64> = mv.iter().copied().filter(|x| *x > 1e-6).collect();
                println!(
                    "  {wname:14} {rounds:3} rounds: links still over by more than {JN_TOL_M} m: {} (worst {:.3} m) | junctions moved {} of {nj} | move m: p50 {:.2} p90 {:.2} max {:.2}; >1 m {}, >2 m {}, >5 m {} | largest move vs the minimax bound of its component: see below",
                    still.len(),
                    quant(&mut resid, 1.0),
                    moved.len(),
                    quant(&mut moved.clone(), 0.5),
                    quant(&mut moved.clone(), 0.9),
                    quant(&mut moved.clone(), 1.0),
                    moved.iter().filter(|x| **x > 1.0).count(),
                    moved.iter().filter(|x| **x > 2.0).count(),
                    moved.iter().filter(|x| **x > 5.0).count()
                );
                if rounds == JN_ROUNDS {
                    // against the minimax
                    let mut worse = 0;
                    for (ci, comp) in comps.iter().enumerate() {
                        let mx = comp.iter().map(|j| mv[*j]).fold(0.0, f64::max);
                        if mx > mm[ci].0 + 0.5 {
                            worse += 1;
                        }
                    }
                    println!(
                        "      components whose largest move exceeds the minimax bound by more than 0.5 m: {worse} of {}",
                        comps.len()
                    );
                    if cname == "PROPOSED" {
                        repaired.insert(
                            if wname == "equal weights" {
                                "equal"
                            } else {
                                "importance"
                            },
                            h.clone(),
                        );
                        // clusters that did not converge
                        if !still.is_empty() {
                            let ov: BTreeSet<usize> = still
                                .iter()
                                .flat_map(|i| [cons[*i].0, cons[*i].1])
                                .collect();
                            let sub: Vec<(usize, usize, f64)> =
                                still.iter().map(|i| cons[*i]).collect();
                            let mut cl = jn_components(nj, &sub);
                            cl.sort_by_key(|c| std::cmp::Reverse(c.len()));
                            println!(
                                "      {} junctions in unresolved links; five largest clusters:",
                                ov.len()
                            );
                            for c in cl.iter().take(5) {
                                let (sx, sy) = c.iter().fold((0.0, 0.0), |a, j| {
                                    (a.0 + juncs[*j].pt.0, a.1 + juncs[*j].pt.1)
                                });
                                let fams: BTreeSet<String> = c
                                    .iter()
                                    .flat_map(|j| {
                                        juncs[*j]
                                            .arms
                                            .iter()
                                            .map(|a| format!("{:?}", lines[a.line].fam))
                                    })
                                    .collect();
                                let kinds: BTreeSet<&str> =
                                    c.iter().map(|j| juncs[*j].kind).collect();
                                let worst = still
                                    .iter()
                                    .filter(|i| c.contains(&cons[**i].0))
                                    .map(|i| (h[cons[*i].1] - h[cons[*i].0]).abs() - cons[*i].2)
                                    .fold(0.0, f64::max);
                                let mmed = c.iter().map(|j| h0[*j]).fold(f64::MIN, f64::max)
                                    - c.iter().map(|j| h0[*j]).fold(f64::MAX, f64::min);
                                println!(
                                    "        {} junctions near ({:.0},{:.0}), families {fams:?}, kinds {kinds:?}, worst residual {worst:.2} m, medians range {mmed:.1} m",
                                    c.len(),
                                    sx / c.len() as f64,
                                    sy / c.len() as f64
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    // sensitivity: junctions whose link is shorter than MERGE_L are one junction
    {
        const MERGE_L: f64 = 3.0;
        let mut rep: Vec<usize> = (0..nj).collect();
        fn root(r: &mut [usize], x: usize) -> usize {
            let mut y = x;
            while r[y] != y {
                y = r[y];
            }
            y
        }
        for l in &jj {
            if l.len <= MERGE_L {
                let (a, b) = (root(&mut rep, l.a.unwrap()), root(&mut rep, l.b.unwrap()));
                if a != b {
                    rep[a.max(b)] = a.min(b);
                }
            }
        }
        let rootv: Vec<usize> = (0..nj).map(|i| root(&mut rep, i)).collect();
        let mut members: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, r) in rootv.iter().enumerate() {
            members.entry(*r).or_default().push(i);
        }
        let merged_groups = members.values().filter(|m| m.len() > 1).count();
        let merged_juncs: usize = members.values().filter(|m| m.len() > 1).map(Vec::len).sum();
        let mut hm = h0.clone();
        let mut rank = vec![5usize; nj];
        for (r, m) in &members {
            let mut v: Vec<f64> = m.iter().map(|i| h0[*i]).collect();
            hm[*r] = jn_med(&mut v);
            rank[*r] = m.iter().map(|i| juncs[*i].rank).min().unwrap_or(5);
        }
        println!(
            "\n  sensitivity: junctions joined by a link of {MERGE_L} m or less (edge to edge) are one junction at the median of their medians: {merged_juncs} junctions in {merged_groups} groups, {} junctions left",
            members.len()
        );
        for (cname, c) in [("PROPOSED", &CEIL_PROPOSED), ("TODAY", &CEIL_TODAY)] {
            let cons: Vec<(usize, usize, f64)> = jj
                .iter()
                .map(|l| {
                    (
                        rootv[l.a.unwrap()],
                        rootv[l.b.unwrap()],
                        ceil_of(l.fam, c) * l.len,
                    )
                })
                .filter(|c| c.0 != c.1)
                .collect();
            let over0 = jn_over(&hm, &cons).len();
            for (wname, mob) in [
                ("equal", vec![1.0; nj]),
                (
                    "importance",
                    rank.iter()
                        .map(|r| JN_RANK_RATIO.powi(*r as i32))
                        .collect::<Vec<f64>>(),
                ),
            ] {
                let h = jn_relax(&hm, &cons, &mob, JN_ROUNDS);
                let still = jn_over(&h, &cons).len();
                let moved: Vec<f64> = members
                    .keys()
                    .map(|i| (h[*i] - hm[*i]).abs())
                    .filter(|x| *x > 1e-6)
                    .collect();
                println!(
                    "    {cname} ceilings, {wname:10}: links {} , over before {over0}, after {JN_ROUNDS} rounds {still} | junctions moved {} | move m p50 {:.2} p90 {:.2} max {:.2}; >1 m {}, >2 m {}, >5 m {}",
                    cons.len(),
                    moved.len(),
                    quant(&mut moved.clone(), 0.5),
                    quant(&mut moved.clone(), 0.9),
                    quant(&mut moved.clone(), 1.0),
                    moved.iter().filter(|x| **x > 1.0).count(),
                    moved.iter().filter(|x| **x > 2.0).count(),
                    moved.iter().filter(|x| **x > 5.0).count()
                );
            }
        }
    }
    // how far the repaired junctions are from the ground median, and the d table
    for (name, h) in &repaired {
        let dist: Vec<f64> = (0..nj).map(|i| (h[i] - h0[i]).abs()).collect();
        jn_line(
            &format!("repair '{name}': |junction height - ground median|, all junctions"),
            dist,
            &[1.0, 2.0, 5.0],
        );
        let cons: Vec<(usize, usize, f64)> = jj
            .iter()
            .map(|l| {
                (
                    l.a.unwrap(),
                    l.b.unwrap(),
                    ceil_of(l.fam, &CEIL_PROPOSED) * l.len,
                )
            })
            .collect();
        let bad: BTreeSet<usize> = jn_over(h, &cons).into_iter().collect();
        let ok: Vec<&Link> = jj
            .iter()
            .copied()
            .enumerate()
            .filter(|(i, _)| !bad.contains(i))
            .map(|(_, l)| l)
            .collect();
        d_report(
            &format!(
                "repair '{name}': d for all links that are legal after the repair (proposed ceilings; {} links left out)",
                bad.len()
            ),
            &ok,
            h,
            &CEIL_PROPOSED,
        );
    }
    // today's pinned ends against the repaired heights
    println!(
        "\n  how close to the ground are the pinned track ends: today's other-road planned height against the ground median at that junction, and the repaired junction height against the same median"
    );
    {
        let mut today: Vec<f64> = Vec::new();
        let mut rep: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        for (ji, j) in juncs.iter().enumerate() {
            // arms that are track ends: today pinned to the most important other road
            for a in &j.arms {
                let l = &lines[a.line];
                if !l.track() || a.end.is_none() {
                    continue;
                }
                // owner height: the planned height of the most important arm that is not this strip
                let owner = j
                    .arms
                    .iter()
                    .filter(|o| o.line != a.line)
                    .min_by_key(|o| fam_rank(fam_of(o)));
                let Some(o) = owner else { continue };
                let ol = &lines[o.line];
                let z = jn_z(&strips[ol.strip], ol, o.station);
                today.push((z - h0[ji]).abs());
                for (name, h) in &repaired {
                    rep.entry(name).or_default().push((h[ji] - h0[ji]).abs());
                }
            }
        }
        jn_line(
            "today: |owner's planned height - ground median|, track ends at junctions",
            today,
            &[2.0, 5.0, 10.0],
        );
        for (k, v) in rep {
            jn_line(
                &format!("repair '{k}': |junction height - ground median|, same ends"),
                v,
                &[2.0, 5.0, 10.0],
            );
        }
    }
    junction_places(strips, base, &lines, &juncs, &links, &h0, &repaired);
    println!(
        "\n#### junction study done {:.1} s",
        t0.elapsed().as_secs_f64()
    );
}

fn junction_places(
    strips: &[DStrip],
    _base: &Heightmap,
    lines: &[Line],
    juncs: &[Junc],
    links: &[Link],
    h0: &[f64],
    repaired: &BTreeMap<&str, Vec<f64>>,
) {
    println!("\n#### junction study E. places by hand");
    let desc = |j: &Junc| -> String {
        j.arms
            .iter()
            .map(|a| {
                let l = &lines[a.line];
                format!(
                    "{:?}{}[strip {} s={:.0}/{:.0} z={:.1}]",
                    l.fam,
                    match a.end {
                        Some(e) => format!(" end{e}"),
                        None => " thru".to_string(),
                    },
                    l.strip,
                    a.station,
                    l.len,
                    jn_z(&strips[l.strip], l, a.station)
                )
            })
            .collect::<Vec<_>>()
            .join(" ; ")
    };
    let places: [(&str, (f64, f64), f64); 3] = [
        ("E1 upper end of the Klingerweg", (3443.0, 3531.0), 45.0),
        ("E2 cluster near (4330, 5455)", (4330.0, 5455.0), 60.0),
        ("E3 dense cluster near (6173, 5641)", (6173.0, 5641.0), 60.0),
    ];
    for (name, c, r) in places {
        println!(
            "\n  == {name}: junctions within {r} m of ({:.0},{:.0})",
            c.0, c.1
        );
        let near: Vec<usize> = (0..juncs.len())
            .filter(|i| (juncs[*i].pt.0 - c.0).hypot(juncs[*i].pt.1 - c.1) <= r)
            .collect();
        for &i in &near {
            let j = &juncs[i];
            let rep: Vec<String> = repaired
                .iter()
                .map(|(k, h)| format!("{k} {:.1}", h[i]))
                .collect();
            println!(
                "    J{i} ({:.1},{:.1}) {} | median R6/10/15 {:.1}/{:.1}/{:.1} point {:.1} | spread R6/10/15 {:.1}/{:.1}/{:.1} | repaired {} | arms: {}",
                j.pt.0,
                j.pt.1,
                j.kind,
                j.med[0],
                j.med[1],
                j.med[2],
                j.point_h,
                j.spread[0],
                j.spread[1],
                j.spread[2],
                rep.join(", "),
                desc(j)
            );
        }
        let set: BTreeSet<usize> = near.iter().copied().collect();
        let mut outside: BTreeSet<usize> = BTreeSet::new();
        for l in links.iter().filter(|l| {
            l.a.is_some_and(|a| set.contains(&a)) || l.b.is_some_and(|b| set.contains(&b))
        }) {
            for j in [l.a, l.b].into_iter().flatten() {
                if !set.contains(&j) {
                    outside.insert(j);
                }
            }
        }
        for &i in &outside {
            let j = &juncs[i];
            println!(
                "    (neighbour outside the radius) J{i} ({:.1},{:.1}) {} | median R6/10/15 {:.1}/{:.1}/{:.1} | spread R10 {:.1} | repaired {} | arms: {}",
                j.pt.0,
                j.pt.1,
                j.kind,
                j.med[0],
                j.med[1],
                j.med[2],
                j.spread[1],
                repaired
                    .iter()
                    .map(|(k, h)| format!("{k} {:.1}", h[i]))
                    .collect::<Vec<_>>()
                    .join(", "),
                desc(j)
            );
        }
        for l in links.iter().filter(|l| {
            l.a.is_some_and(|a| set.contains(&a)) || l.b.is_some_and(|b| set.contains(&b))
        }) {
            let c27 = ceil_of(l.fam, &CEIL_PROPOSED);
            let line = &lines[l.line];
            match (l.a, l.b) {
                (Some(a), Some(b)) => {
                    let dh = h0[b] - h0[a];
                    let need = dh.abs() / l.len;
                    println!(
                        "    link strip {} {:?} J{a}->J{b} stations {:.0}..{:.0} L {:.1} m, medians {:.1} -> {:.1}, needs {:.0} % (ceiling {:.0} %, today's {:.1} %) excess {:.2} m; ground at the link ends {:.1}/{:.1}, today's planned {:.1}/{:.1}",
                        line.strip,
                        l.fam,
                        l.from,
                        l.to,
                        l.len,
                        h0[a],
                        h0[b],
                        need * 100.0,
                        c27 * 100.0,
                        ceil_of(l.fam, &CEIL_TODAY) * 100.0,
                        (dh.abs() - c27 * l.len).max(0.0),
                        l.g[0],
                        l.g[l.g.len() - 1],
                        l.zp[0],
                        l.zp[l.zp.len() - 1]
                    );
                    for g in [0.27, 0.36, 0.45] {
                        let d = jn_d(l, g, Some(h0[a]), Some(h0[b]));
                        let dp = jn_d(l, g, Some(l.zp[0]), Some(l.zp[l.zp.len() - 1]));
                        let df = jn_d(l, g, None, None);
                        println!(
                            "        d at {:.0} %: ends on medians {} | ends at today's planned {} | free {}",
                            g * 100.0,
                            d.map_or("not joinable".to_string(), |x| format!("{x:.2} m")),
                            dp.map_or("not joinable".to_string(), |x| format!("{x:.2} m")),
                            df.map_or("not joinable".to_string(), |x| format!("{x:.2} m"))
                        );
                    }
                    for (k, h) in repaired {
                        println!(
                            "        repaired '{k}': heights {:.1} -> {:.1} (needs {:.0} %), d at the ceiling {}",
                            h[a],
                            h[b],
                            (h[b] - h[a]).abs() / l.len * 100.0,
                            jn_d(l, c27, Some(h[a]), Some(h[b]))
                                .map_or("not joinable".to_string(), |x| format!("{x:.2} m"))
                        );
                    }
                }
                (a, b) => {
                    let (end, j) = match (a, b) {
                        (Some(j), None) => (l.g.len() - 1, j),
                        (None, Some(j)) => (0, j),
                        _ => continue,
                    };
                    let _ = end;
                    println!(
                        "    free-end link strip {} {:?} from J{j} stations {:.0}..{:.0} L {:.1} m; median {:.1}",
                        line.strip, l.fam, l.from, l.to, l.len, h0[j]
                    );
                }
            }
        }
    }
    // the Klingerweg
    println!("\n  == E1 the Klingerweg, every link of the strip");
    if let Some(li) = lines
        .iter()
        .position(|l| l.ids.iter().any(|i| i == KLINGER))
    {
        let line = &lines[li];
        println!(
            "    strip {} {:?}, {:.0} m, {} joints",
            line.strip,
            line.fam,
            line.len,
            line.jp.len()
        );
        for l in links.iter().filter(|l| l.line == li) {
            let pa = l.a.map(|a| h0[a]);
            let pb = l.b.map(|b| h0[b]);
            println!(
                "    link stations {:.0}..{:.0} (L {:.0} m) ends: {:?} / {:?}, ground at the ends {:.1} / {:.1}, junction medians {:?} / {:?}, today's planned {:.1} / {:.1}",
                l.from,
                l.to,
                l.len,
                l.a.map(|a| juncs[a].pt),
                l.b.map(|b| juncs[b].pt),
                l.g[0],
                l.g[l.g.len() - 1],
                pa,
                pb,
                l.zp[0],
                l.zp[l.zp.len() - 1]
            );
            for g in [0.27, 0.36, 0.45, 0.60] {
                let d = jn_d(l, g, pa, pb);
                let dp = jn_d(l, g, Some(l.zp[0]), Some(l.zp[l.zp.len() - 1]));
                let df = jn_d(l, g, None, None);
                println!(
                    "      d at {:.0} %: ends on junction medians (free where no junction) {} | ends at today's planned {} | free {}",
                    g * 100.0,
                    d.map_or("not joinable".to_string(), |x| format!("{x:.2} m")),
                    dp.map_or("not joinable".to_string(), |x| format!("{x:.2} m")),
                    df.map_or("not joinable".to_string(), |x| format!("{x:.2} m"))
                );
            }
        }
    } else {
        println!("    Klingerweg strip not found");
    }
}

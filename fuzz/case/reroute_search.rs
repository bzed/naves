// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

// One case of the `reroute_search` fuzz target, shared (with `include!`) by the target and by the seed test in
// `crates/naves-core/tests/fuzz_seeds.rs`, so that the two cannot drift apart.
//
// The input is a header of 57 bytes, then records of 17 bytes. The header: six little-endian `f64` (the start
// anchor's x, y and heading, the end anchor's x, y and heading), one more `f64` (a radius, used only if flag
// bit 5 is set), and a flag byte: bit 0 the end has no heading, bit 1 the start has none, bit 2 the start has a
// junction straight, bit 3 the end has one, bit 4 the relaxed search, bit 5 the radius of the header, bit 6 no
// terrain outside the map. A record: x and y as `f64`, then a kind byte: 0 a vertex of the original line, 1 a
// vertex of a water polygon, anything else a vertex of a wall (a stream that may be crossed once).

mod reroute_search_case {
    use naves_core::reroute::band::Band;
    use naves_core::reroute::field::Field;
    use naves_core::reroute::raster::{Clearance, Obstacles, Wall, WallKind};
    use naves_core::reroute::search::{
        LegInput, LegOutcome, Weights, search_leg, search_leg_relaxed, self_clearance,
    };
    use naves_core::reroute::{Anchor, RerouteParams};

    fn number(head: &[u8], index: usize) -> f64 {
        head.get(index * 8..index * 8 + 8)
            .and_then(|b| b.try_into().ok())
            .map_or(0.0, f64::from_le_bytes)
    }

    fn inside(poly: &[(f64, f64)], p: (f64, f64)) -> bool {
        let mut odd = false;
        for (i, a) in poly.iter().enumerate() {
            let Some(b) = poly.get((i + 1) % poly.len()) else {
                continue;
            };
            if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
                odd = !odd;
            }
        }
        odd
    }

    /// The records: the original line, the water polygon and the wall.
    fn decode(rest: &[u8]) -> [Vec<(f64, f64)>; 3] {
        let [mut original, mut polygon, mut wall] = <[Vec<(f64, f64)>; 3]>::default();
        for c in rest.as_chunks::<17>().0 {
            let (Some(x), Some(y), Some(&kind)) = (c.get(0..8), c.get(8..16), c.get(16)) else {
                continue;
            };
            let (Ok(x), Ok(y)) = (x.try_into(), y.try_into()) else {
                continue;
            };
            let p = (f64::from_le_bytes(x), f64::from_le_bytes(y));
            match kind {
                0 => original.push(p),
                1 => polygon.push(p),
                _ => wall.push(p),
            }
        }
        [original, polygon, wall]
    }

    /// Runs the case; the number of routes found (0 to 2).
    pub fn run(data: &[u8]) -> usize {
        let (head, rest) = data.split_at_checked(57).unwrap_or((data, &[]));
        let flags = head.get(56).copied().unwrap_or(0);
        let [original, polygon, wall] = decode(rest);
        let anchor = |at: (f64, f64), heading: f64, none: bool, straight: bool| Anchor {
            at,
            heading: (!none).then_some(heading),
            straight_m: if straight { 6.25 } else { 0.0 },
        };
        let start = anchor(
            (number(head, 0), number(head, 1)),
            number(head, 2),
            flags & 2 != 0,
            flags & 4 != 0,
        );
        let end = anchor(
            (number(head, 3), number(head, 4)),
            number(head, 5),
            flags & 1 != 0,
            flags & 8 != 0,
        );
        let radius = if flags & 32 != 0 { number(head, 6) } else { 10.5 };
        let params = RerouteParams::default();
        let mut o = Obstacles::new((-50.0, -50.0), (250.0, 250.0), 4.0, 200.0, 3.0, 20_000);
        o.stamp_water(&|p| inside(&polygon, p));
        o.set_original(&original);
        o.finish();
        let walls = [Wall {
            line: wall,
            allowed: usize::from(flags & 16 != 0),
            kind: WallKind::Stream,
            anchors: [start.at, end.at],
            end_m: 10.0,
        }];
        let clear = Clearance::new(5.0, 12.0).unwrap_or(Clearance::NONE);
        let band = Band::from_params(&params, 0.2, 4.0);
        let ground = |p: (f64, f64)| {
            let inside_map = (0.0..=200.0).contains(&p.0) && (0.0..=200.0).contains(&p.1);
            (inside_map || flags & 64 == 0).then_some(100.0 + 0.1 * p.0 - 0.05 * p.1)
        };
        let field = Field::build(
            (-50.0, -50.0),
            (250.0, 250.0),
            end.at,
            6.0,
            4.0,
            &band,
            &ground,
            &o,
            &walls,
            clear,
            31.5,
            100_000,
        );
        let input = LegInput {
            start,
            end,
            radius,
            step_m: 6.0,
            band: &band,
            ground: &ground,
            obstacles: &o,
            walls: &walls,
            clear,
            field: &field,
            weights: Weights::from_params(&params),
            stray_cap_m: 31.5,
            arrive_reach_m: 42.0,
            self_along_m: 58.0,
            self_clear_m: self_clearance(&original, 58.0, 12.0, 6.0),
            original: &original,
            check_radius_m: radius * 0.99 / 1.05,
            budgets: &[(1.0, 40), (1.5, 40)],
        };
        let outcomes = [
            search_leg(&input),
            if flags & 16 != 0 {
                search_leg_relaxed(&input, &[0.1, 0.2, f64::from(flags)])
            } else {
                search_leg_relaxed(&input, &[0.2, 0.36])
            },
        ];
        let mut found = 0;
        for outcome in outcomes.into_iter().flatten() {
            if let LegOutcome::Found(route) = outcome {
                assert!(route.points.iter().all(|p| p.0.is_finite() && p.1.is_finite()));
                found += 1;
            }
        }
        found
    }
}

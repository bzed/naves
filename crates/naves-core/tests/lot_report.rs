// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the lot stage reports: the one-line summary and the findings.

#![expect(
    clippy::expect_used,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::lot::{
    Assessed, Form, LotArea, LotCounts, LotKind, LotReport, LotSurface, NO_GROUP, Outcome,
    Placement, Surface, SurfaceSource, centre,
};
use naves_core::netclean::{CleanReport, CutAccess, DroppedInLot};
use naves_core::surface::SurfaceClass;
use naves_core::validate::{Rule, Violation, lot_violations};

fn ring(x0: f64, y0: f64) -> Vec<(f64, f64)> {
    vec![
        (x0, y0),
        (x0 + 100.0, y0),
        (x0 + 100.0, y0 + 100.0),
        (x0, y0 + 100.0),
        (x0, y0),
    ]
}

fn area(id: &str, kind: LotKind, x0: f64) -> LotArea {
    LotArea {
        id: id.to_string(),
        kind,
        placement: Placement::Ground,
        outer: ring(x0, 0.0),
        holes: Vec::new(),
        area_m2: 10_000.0,
        surface_tag: None,
        holes_dropped: 0,
    }
}

fn smoothed(id: &str, kind: LotKind, x0: f64, form: Form, group: usize) -> Assessed {
    Assessed {
        area: area(id, kind, x0),
        group,
        outcome: Outcome::Smoothed { form },
        steep: false,
        move_p95_m: 0.5,
        move_max_m: 1.0,
        slope_p95: 0.1,
        relief_before_m: 0.4,
        relief_after_m: 0.1,
        range_before: (100.0, 101.0),
        range_after: (100.2, 101.0),
        water_share: 0.0,
    }
}

fn unsmoothed(id: &str, kind: LotKind, x0: f64, outcome: Outcome) -> Assessed {
    Assessed {
        area: area(id, kind, x0),
        group: NO_GROUP,
        outcome,
        steep: false,
        move_p95_m: f64::NAN,
        move_max_m: f64::NAN,
        slope_p95: f64::NAN,
        relief_before_m: f64::NAN,
        relief_after_m: f64::NAN,
        range_before: (f64::NAN, f64::NAN),
        range_after: (f64::NAN, f64::NAN),
        water_share: f64::NAN,
    }
}

fn surface(group: usize, x0: f64, form: Form, lambda: f64) -> LotSurface {
    LotSurface {
        group,
        members: vec![format!("osm/way/{group}")],
        polygons: vec![(ring(x0, 0.0), Vec::new())],
        surface: Surface {
            at: (x0 + 50.0, 50.0),
            coeffs: [100.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            form,
            lambda,
        },
    }
}

/// The fixture of the brief: five lots (three car parks, two camp sites), four smoothed (three plane, one quadratic,
/// three paved and one grass), one steep, none move limited, one surface only, none excluded; three roads (536 m)
/// removed and one access road cut.
fn fixture() -> (LotReport, CleanReport) {
    let assessed = vec![
        smoothed("osm/way/1", LotKind::CarPark, 0.0, Form::Plane, 0),
        smoothed("osm/way/2", LotKind::CarPark, 200.0, Form::Plane, 1),
        smoothed("osm/way/3", LotKind::CarPark, 400.0, Form::Quadratic, 2),
        smoothed("osm/way/4", LotKind::CampSite, 600.0, Form::Plane, 3),
        unsmoothed(
            "osm/way/5",
            LotKind::CampSite,
            800.0,
            Outcome::SurfaceOnly {
                reason: "street_side",
            },
        ),
    ];
    let classes = vec![
        ("osm/way/1".to_string(), SurfaceClass::Tarmac),
        ("osm/way/2".to_string(), SurfaceClass::Tarmac),
        ("osm/way/3".to_string(), SurfaceClass::Tarmac),
        ("osm/way/4".to_string(), SurfaceClass::Grass),
        ("osm/way/5".to_string(), SurfaceClass::Grass),
    ];
    let sources = vec![
        SurfaceSource::Tag,
        SurfaceSource::Roads,
        SurfaceSource::Roads,
        SurfaceSource::Unserved,
        SurfaceSource::Street,
    ];
    let surfaces = vec![
        surface(0, 0.0, Form::Plane, 1.0),
        surface(1, 200.0, Form::Plane, 1.0),
        surface(2, 400.0, Form::Quadratic, 1.0),
        surface(3, 600.0, Form::Plane, 1.0),
    ];
    let mut report = LotReport {
        assessed,
        surfaces,
        classes,
        sources,
    };
    let first = report.assessed.first_mut().expect("the first lot");
    first.steep = true;
    first.slope_p95 = 0.31;
    let clean = CleanReport {
        lot_roads: vec![
            DroppedInLot {
                stretch: "1:1-2".to_string(),
                lot: "osm/way/1".to_string(),
                length_m: 300.0,
                at: (10.0, 10.0),
            },
            DroppedInLot {
                stretch: "2:2-3".to_string(),
                lot: "osm/way/1".to_string(),
                length_m: 180.0,
                at: (20.0, 20.0),
            },
            DroppedInLot {
                stretch: "3:3-4".to_string(),
                lot: "osm/way/4".to_string(),
                length_m: 56.0,
                at: (30.0, 30.0),
            },
        ],
        lot_access: vec![CutAccess {
            stretch: "4:4-5".to_string(),
            lot: "osm/way/2".to_string(),
            cut_m: 12.0,
            entry: (210.0, 10.0),
            node: 99,
        }],
        junctions_in_lots: 4,
        ..CleanReport::default()
    };
    (report, clean)
}

#[test]
fn the_summary_is_one_line_with_the_counts_of_the_fixture() {
    let (report, clean) = fixture();
    assert_eq!(
        report.summary(&clean),
        "Lots: 5 (3 car parks, 2 camp sites), 4 smoothed (3 plane, 1 quadratic; 3 paved, 1 grass; 1 steep, 0 move \
         limited), 1 surface only, 0 excluded; 3 roads (536 m) removed, 1 access road cut"
    );
}

#[test]
fn the_summary_of_no_lot_says_none() {
    let report = LotReport::default();
    assert_eq!(report.summary(&CleanReport::default()), "Lots: none");
    assert_eq!(report.counts(), LotCounts::default());
}

#[test]
fn the_counts_name_the_kinds_the_outcomes_and_the_reasons() {
    let (report, _) = fixture();
    let counts = report.counts();
    assert_eq!(counts.total, 5);
    assert_eq!(
        counts.kinds,
        [(LotKind::CarPark, 3), (LotKind::CampSite, 2)]
    );
    assert_eq!(counts.forms, [(Form::Plane, 3), (Form::Quadratic, 1)]);
    assert_eq!(
        (counts.smoothed, counts.surface_only, counts.excluded),
        (4, 1, 0)
    );
    assert_eq!(counts.reasons, [("street_side", 1)]);
    // paved and grass count every lot that gets a surface, a surface-only lot included
    assert_eq!((counts.paved, counts.grass), (3, 2));
    assert_eq!((counts.steep, counts.move_limited), (1, 0));
    assert_eq!(
        counts.outcomes,
        [
            (LotKind::CarPark, Outcome::Smoothed { form: Form::Plane }, 2),
            (
                LotKind::CarPark,
                Outcome::Smoothed {
                    form: Form::Quadratic
                },
                1
            ),
            (
                LotKind::CampSite,
                Outcome::Smoothed { form: Form::Plane },
                1
            ),
            (
                LotKind::CampSite,
                Outcome::SurfaceOnly {
                    reason: "street_side"
                },
                1
            ),
        ]
    );
}

#[test]
fn the_report_finds_the_group_surface_and_the_class_of_a_lot() {
    let (report, _) = fixture();
    let lot = &report.assessed[2];
    let group = report.surface_of(lot).expect("a surface");
    assert_eq!((group.group, group.surface.lambda), (2, 1.0));
    assert_eq!(report.class_of("osm/way/3"), Some(SurfaceClass::Tarmac));
    assert_eq!(report.source_of("osm/way/4"), Some(SurfaceSource::Unserved));
    assert_eq!(report.move_limited(&report.assessed[0]), None);
    let nowhere = unsmoothed(
        "osm/way/9",
        LotKind::CarPark,
        900.0,
        Outcome::Excluded { reason: "in_water" },
    );
    assert!(report.surface_of(&nowhere).is_none());
    assert!(report.class_of("osm/way/9").is_none());
    assert!(report.source_of("osm/way/9").is_none());
}

#[test]
fn every_outcome_has_a_finding_at_the_lots_centre_with_its_numbers() {
    let assessed = vec![
        smoothed("osm/way/1", LotKind::CarPark, 0.0, Form::Plane, 0),
        {
            let mut steep = smoothed("osm/way/2", LotKind::CampSite, 200.0, Form::Quadratic, 1);
            steep.steep = true;
            steep.slope_p95 = 0.33;
            steep
        },
        smoothed("osm/way/3", LotKind::CaravanSite, 400.0, Form::Plane, 2),
        unsmoothed(
            "osm/way/4",
            LotKind::CarPark,
            600.0,
            Outcome::SurfaceOnly {
                reason: "street_side",
            },
        ),
        unsmoothed(
            "osm/way/5",
            LotKind::BusParking,
            800.0,
            Outcome::Excluded {
                reason: "not_at_ground",
            },
        ),
    ];
    let report = LotReport {
        assessed,
        surfaces: vec![
            surface(0, 0.0, Form::Plane, 1.0),
            surface(1, 200.0, Form::Quadratic, 1.0),
            // the third lot is move limited: its change is scaled by λ
            surface(2, 400.0, Form::Plane, 0.75),
        ],
        classes: vec![
            ("osm/way/1".to_string(), SurfaceClass::Tarmac),
            ("osm/way/2".to_string(), SurfaceClass::Grass),
            ("osm/way/3".to_string(), SurfaceClass::Tarmac),
            ("osm/way/4".to_string(), SurfaceClass::Tarmac),
        ],
        sources: vec![SurfaceSource::Tag; 4],
    };
    let clean = CleanReport {
        lot_roads: vec![DroppedInLot {
            stretch: "1:1-2".to_string(),
            lot: "osm/way/1".to_string(),
            length_m: 42.0,
            at: (123.0, 456.0),
        }],
        lot_access: vec![CutAccess {
            stretch: "2:2-3".to_string(),
            lot: "osm/way/2".to_string(),
            cut_m: 6.0,
            entry: (12.0, 34.0),
            node: 7,
        }],
        ..CleanReport::default()
    };
    let found = lot_violations(&report, &clean);
    let of = |rule: Rule| -> Vec<&Violation> { found.iter().filter(|v| v.rule == rule).collect() };
    // one smoothed finding per smoothed lot, at its centre
    let smoothed = of(Rule::LotSmoothed);
    assert_eq!(smoothed.len(), 3);
    for (finding, lot) in smoothed.iter().zip(&report.assessed) {
        assert_eq!(finding.at, centre(&lot.area));
    }
    assert!(smoothed[0].message.contains("plane"), "{:?}", smoothed[0]);
    assert!(smoothed[0].message.contains("paved"), "{:?}", smoothed[0]);
    assert!(smoothed[0].message.contains("1.00 m"), "{:?}", smoothed[0]);
    assert!(
        smoothed[1].message.contains("quadratic"),
        "{:?}",
        smoothed[1]
    );
    assert!(smoothed[1].message.contains("grass"), "{:?}", smoothed[1]);
    // steep: its slope against the 20 % a vehicle can take
    let steep = of(Rule::LotSteep);
    assert_eq!(steep.len(), 1, "{found:?}");
    assert_eq!(steep[0].at, centre(&report.assessed[1].area));
    assert!(steep[0].message.contains("33 %"), "{:?}", steep[0]);
    assert!(steep[0].message.contains("20 %"), "{:?}", steep[0]);
    // move limited: the move it would have needed (3 / 0.75 = 4 m) against the 3 m
    let limited = of(Rule::LotMoveLimited);
    assert_eq!(limited.len(), 1, "{found:?}");
    assert!(limited[0].message.contains("4.00 m"), "{:?}", limited[0]);
    assert!(limited[0].message.contains("3.0 m"), "{:?}", limited[0]);
    assert!(limited[0].message.contains("0.75"), "{:?}", limited[0]);
    // surface only and excluded carry their reason
    let only = of(Rule::LotSurfaceOnly);
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].at, centre(&report.assessed[3].area));
    assert!(only[0].message.contains("street_side"), "{:?}", only[0]);
    let excluded = of(Rule::LotExcluded);
    assert_eq!(excluded.len(), 1);
    assert!(
        excluded[0].message.contains("not_at_ground"),
        "{excluded:?}"
    );
    // the removed stretch is a finding at its middle, the cut access road at its new end
    let roads = of(Rule::RoadInLotDropped);
    assert_eq!(roads.len(), 2, "{found:?}");
    assert_eq!(roads[0].at, (123.0, 456.0));
    assert!(roads[0].message.contains("1:1-2"), "{:?}", roads[0]);
    assert_eq!(roads[1].at, (12.0, 34.0));
    assert!(roads[1].message.contains("access road"), "{:?}", roads[1]);
}

#[test]
fn every_kind_and_every_outcome_has_its_stable_name() {
    assert_eq!(
        LotKind::ALL.map(LotKind::id),
        [
            "car_park",
            "bus_parking",
            "rest_area",
            "camp_site",
            "caravan_site"
        ]
    );
    assert_eq!(
        LotKind::ALL.map(LotKind::words),
        [
            ("car park", "car parks"),
            ("coach park", "coach parks"),
            ("rest area", "rest areas"),
            ("camp site", "camp sites"),
            ("caravan site", "caravan sites")
        ]
    );
    assert_eq!(Form::Plane.id(), "plane");
    assert_eq!(Form::Quadratic.id(), "quadratic");
    assert_eq!(Outcome::Smoothed { form: Form::Plane }.id(), "smoothed");
    assert_eq!(
        Outcome::SurfaceOnly {
            reason: "no_vertex"
        }
        .id(),
        "surface_only"
    );
    assert_eq!(Outcome::Excluded { reason: "in_water" }.id(), "excluded");
    assert_eq!(Outcome::Smoothed { form: Form::Plane }.reason(), None);
    assert_eq!(SurfaceSource::Tag.id(), "tag");
    assert_eq!(SurfaceSource::Roads.id(), "roads");
    assert_eq!(SurfaceSource::Street.id(), "street");
    assert_eq!(SurfaceSource::Unserved.id(), "none");
    assert_eq!(
        Outcome::Excluded { reason: "in_water" }.reason(),
        Some("in_water")
    );
}

#[test]
fn the_centre_of_a_lot_without_an_outer_ring_is_the_origin() {
    // a lot `areas` would have skipped; the findings must not panic on a hand-built one
    let mut empty = area("osm/way/1", LotKind::CarPark, 0.0);
    empty.outer.clear();
    assert_eq!(centre(&empty), (0.0, 0.0));
    // a ring without its closing point gives the same centre
    let mut open = area("osm/way/2", LotKind::RestArea, 0.0);
    open.outer.pop();
    assert_eq!(centre(&open), (50.0, 50.0));
}

#[test]
fn the_counts_cover_the_excluded_reasons() {
    let report = LotReport {
        assessed: vec![
            unsmoothed(
                "osm/way/1",
                LotKind::BusParking,
                0.0,
                Outcome::Excluded {
                    reason: "not_at_ground",
                },
            ),
            unsmoothed(
                "osm/way/2",
                LotKind::RestArea,
                200.0,
                Outcome::Excluded { reason: "in_water" },
            ),
        ],
        ..LotReport::default()
    };
    let counts = report.counts();
    assert_eq!((counts.total, counts.excluded), (2, 2));
    assert_eq!(counts.reasons, [("not_at_ground", 1), ("in_water", 1)]);
    assert_eq!(
        counts.outcomes,
        [
            (
                LotKind::BusParking,
                Outcome::Excluded {
                    reason: "not_at_ground"
                },
                1
            ),
            (
                LotKind::RestArea,
                Outcome::Excluded { reason: "in_water" },
                1
            ),
        ]
    );
    // no lot has a surface, so none is paved or grass
    assert_eq!((counts.paved, counts.grass), (0, 0));
}

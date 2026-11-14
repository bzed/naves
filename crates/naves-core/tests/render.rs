// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Top-down rendering of a heightmap.

#![expect(
    clippy::unwrap_used,
    reason = "test helper code; failing loudly is the point"
)]

use naves_core::heightmap::{Heightmap, MapGeo};
use naves_core::imagery::{Composite, ImageryRaster, TileImagery, merge};
use naves_core::project::Project;
use naves_core::render::{Overlay, RenderError, RenderRequest, render_topdown};
use naves_core::surface::{SurfaceClass, SurfaceGrid};

/// A `size` x `size` heightmap with 10 m cells whose height is `f(col, row)`.
fn map(size: u8, f: impl Fn(f32, f32) -> f32) -> Heightmap {
    let h = (0..size)
        .flat_map(|row| (0..size).map(move |col| (f32::from(col), f32::from(row))))
        .map(|(c, r)| f(c, r))
        .collect();
    Heightmap {
        size: u32::from(size),
        cell_m: 10.0,
        h,
    }
}

fn request(hm: &Heightmap, px: u32, overlay: Overlay) -> RenderRequest {
    let extent = f64::from(hm.size) * f64::from(hm.cell_m);
    RenderRequest {
        x0: 0.0,
        y0: 0.0,
        x1: extent,
        y1: extent,
        px,
        overlay,
    }
}

#[test]
fn north_up() {
    // rising to the north: the top image row is the highest ground
    let hm = map(64, |_, row| row * 3.0);
    let img = render_topdown(&hm, &request(&hm, 64, Overlay::Height), None, None).unwrap();
    let top = img.get_pixel(32, 0);
    let bottom = img.get_pixel(32, img.height() - 1);
    assert!(top[0] > bottom[0] + 100, "{top:?} vs {bottom:?}");
}

#[test]
fn east_is_right() {
    let hm = map(64, |col, _| col * 3.0);
    let img = render_topdown(&hm, &request(&hm, 64, Overlay::Height), None, None).unwrap();
    assert!(img.get_pixel(img.width() - 1, 32)[0] > img.get_pixel(0, 32)[0] + 100);
}

#[test]
fn aspect_kept() {
    let hm = map(64, |_, _| 1.0);
    let req = RenderRequest {
        x0: 0.0,
        y0: 0.0,
        x1: 2000.0,
        y1: 1000.0,
        px: 400,
        overlay: Overlay::Hillshade,
    };
    let img = render_topdown(&hm, &req, None, None).unwrap();
    assert_eq!((img.width(), img.height()), (400, 200));
}

#[test]
fn slope_overlay_marks_steep() {
    // flat for 32 columns, then a 60 degree wall: rises 10 * tan(60) per 10 m cell
    let hm = map(64, |col, _| {
        if col < 32.0 {
            0.0
        } else {
            (col - 32.0) * 10.0 * 3.0_f32.sqrt()
        }
    });
    let img = render_topdown(&hm, &request(&hm, 64, Overlay::Slope), None, None).unwrap();
    let wall = img.get_pixel(50, 20);
    let flat = img.get_pixel(10, 20);
    assert!(
        wall[0] > wall[1] && wall[0] > wall[2],
        "wall should be red-dominant: {wall:?}"
    );
    assert!(
        flat[1] > flat[0],
        "flat ground should be green-dominant: {flat:?}"
    );
}

#[test]
fn hillshade_is_lit_from_the_north_west() {
    // a slope that faces north-west (rises towards the south-east) is lit, the opposite one is dark
    let faces_nw = map(32, |col, row| (col - row) * 4.0);
    let faces_se = map(32, |col, row| (row - col) * 4.0);
    let lit = render_topdown(
        &faces_nw,
        &request(&faces_nw, 32, Overlay::Hillshade),
        None,
        None,
    )
    .unwrap();
    let dark = render_topdown(
        &faces_se,
        &request(&faces_se, 32, Overlay::Hillshade),
        None,
        None,
    )
    .unwrap();
    assert!(lit.get_pixel(16, 16)[0] > dark.get_pixel(16, 16)[0] + 50);
}

#[test]
fn flat_height_map_does_not_divide_by_zero() {
    let hm = map(16, |_, _| 5.0);
    let img = render_topdown(&hm, &request(&hm, 16, Overlay::Height), None, None).unwrap();
    assert_eq!(img.get_pixel(3, 3)[3], 255);
}

#[test]
fn bad_requests_are_errors() {
    let hm = map(16, |_, _| 5.0);
    let ok = request(&hm, 16, Overlay::Height);
    for bad in [
        RenderRequest { x1: -1.0, ..ok },
        RenderRequest { y1: ok.y0, ..ok },
        RenderRequest { x0: f64::NAN, ..ok },
        RenderRequest {
            y1: f64::INFINITY,
            ..ok
        },
        RenderRequest { px: 0, ..ok },
        RenderRequest { px: 100_000, ..ok },
        RenderRequest { y1: 1.0e9, ..ok },
    ] {
        assert!(
            matches!(
                render_topdown(&hm, &bad, None, None),
                Err(RenderError::Bounds | RenderError::Width(_) | RenderError::Aspect)
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn output_is_deterministic() {
    let hm = map(32, |col, row| col * row * 0.1);
    let a = render_topdown(&hm, &request(&hm, 48, Overlay::Slope), None, None).unwrap();
    let b = render_topdown(&hm, &request(&hm, 48, Overlay::Slope), None, None).unwrap();
    assert_eq!(a, b);
}

#[test]
fn surface_overlay_uses_the_class_colours() {
    // a 100 m map in quadrants: grass south-west, rock south-east, forest north-west, soil north-east
    let hm = map(10, |_, _| 100.0);
    let grid = SurfaceGrid::from_fn(10, 10.0, |col, row| match (col >= 5, row >= 5) {
        (false, false) => SurfaceClass::Grass,
        (true, false) => SurfaceClass::Rock,
        (false, true) => SurfaceClass::Forest,
        (true, true) => SurfaceClass::Soil,
    });
    let img = render_topdown(&hm, &request(&hm, 40, Overlay::Surface), Some(&grid), None).unwrap();
    // the image is north up: row 0 is the north
    assert_eq!(img.get_pixel(10, 30).0, [0x4a, 0x7a, 0x3a, 255], "grass");
    assert_eq!(img.get_pixel(30, 30).0, [0x8a, 0x87, 0x80, 255], "rock");
    assert_eq!(img.get_pixel(10, 10).0, [0x2e, 0x4d, 0x2a, 255], "forest");
    assert_eq!(img.get_pixel(30, 10).0, [0x7a, 0x68, 0x4e, 255], "soil");
}

/// A project of 1280 real metres (a 640 m map, like `map(64, ..)`) and imagery over its western half.
fn imagery_over_the_west() -> (Project, Composite) {
    let mut project = Project::new("r", 47.0, 12.0, 1280.0);
    project.imagery.brightness = 1.0; // the tests compare the colours of the file
    let geo = MapGeo::new(&project).unwrap();
    let (south, west) = geo.to_geo(-50.0, -50.0).unwrap();
    let (north, east) = geo.to_geo(690.0, 690.0).unwrap();
    let raster = ImageryRaster::from_fn(west, south, east, north, 1e-4, |lat, lon| {
        let (x, _) = geo.to_map(lat, lon).unwrap();
        (x < 320.0).then_some([200, 100, 50])
    });
    let (composite, _) = merge(vec![raster], &project).unwrap();
    (project, composite)
}

#[test]
fn imagery_overlay_shows_the_composite() {
    let hm = map(64, |_, _| 100.0);
    let (project, composite) = imagery_over_the_west();
    let imagery = TileImagery {
        composite: &composite,
        project: &project,
    };
    let img = render_topdown(
        &hm,
        &request(&hm, 64, Overlay::Imagery),
        None,
        Some(&imagery),
    )
    .unwrap();
    assert_eq!(img.get_pixel(10, 32).0, [200, 100, 50, 255]);
}

#[test]
fn imagery_overlay_follows_the_brightness() {
    // the preview shows what the build will make: the colours times [imagery] brightness, clamped
    let hm = map(64, |_, _| 100.0);
    let (mut project, _) = imagery_over_the_west();
    project.imagery.brightness = 1.5;
    let geo = MapGeo::new(&project).unwrap();
    let (south, west) = geo.to_geo(-50.0, -50.0).unwrap();
    let (north, east) = geo.to_geo(690.0, 690.0).unwrap();
    let raster =
        ImageryRaster::from_fn(west, south, east, north, 1e-4, |_, _| Some([200, 100, 50]));
    let (composite, _) = merge(vec![raster], &project).unwrap();
    let imagery = TileImagery {
        composite: &composite,
        project: &project,
    };
    let img = render_topdown(
        &hm,
        &request(&hm, 64, Overlay::Imagery),
        None,
        Some(&imagery),
    )
    .unwrap();
    assert_eq!(img.get_pixel(10, 32).0, [255, 150, 75, 255]);
}

#[test]
fn imagery_overlay_marks_gaps() {
    let hm = map(64, |_, _| 100.0);
    let (project, composite) = imagery_over_the_west();
    let imagery = TileImagery {
        composite: &composite,
        project: &project,
    };
    let img = render_topdown(
        &hm,
        &request(&hm, 64, Overlay::Imagery),
        None,
        Some(&imagery),
    )
    .unwrap();
    // no imagery in the east: a checkered grey that cannot be mistaken for ground
    let (a, b) = (img.get_pixel(50, 10).0, img.get_pixel(58, 10).0);
    assert!(
        a[0] == a[1] && a[1] == a[2] && b[0] == b[1] && b[1] == b[2],
        "{a:?} {b:?}"
    );
    assert_ne!(a, b, "neighbouring squares differ");
}

#[test]
fn imagery_overlay_without_sources_is_an_error() {
    let hm = map(10, |_, _| 100.0);
    assert_eq!(
        render_topdown(&hm, &request(&hm, 40, Overlay::Imagery), None, None).unwrap_err(),
        RenderError::NoImagery
    );
}

#[test]
fn surface_overlay_needs_surfaces() {
    let hm = map(10, |_, _| 100.0);
    assert_eq!(
        render_topdown(&hm, &request(&hm, 40, Overlay::Surface), None, None).unwrap_err(),
        RenderError::NoSurfaces
    );
}

// ---- map layers: images over a rectangle of the map ----

use naves_core::render::{LayerData, MapLayer, render_layer};

/// A 64 x 64 map (640 m) with a 200 m high step between the columns 31 and 32.
fn wall() -> Heightmap {
    map(64, |col, _| if col >= 32.0 { 200.0 } else { 0.0 })
}

fn whole(hm: &Heightmap, px: u32) -> RenderRequest {
    request(hm, px, Overlay::Hillshade)
}

#[test]
fn each_layer_is_transparent_where_it_draws_nothing() {
    let hm = map(64, |_, _| 100.0);
    for layer in MapLayer::ALL {
        let img = render_layer(*layer, &LayerData::new(&hm), &whole(&hm, 128)).unwrap();
        assert!(img.pixels().all(|p| p[3] == 0), "{layer:?}");
    }
}

#[test]
fn hatching_marks_only_ground_above_the_limit() {
    let hm = wall();
    // 4 px per 10 m cell; the step is in x 310 m to 320 m
    let img = render_layer(
        MapLayer::TooSteepToWalk,
        &LayerData::new(&hm),
        &RenderRequest {
            x0: 200.0,
            y0: 200.0,
            x1: 440.0,
            y1: 440.0,
            px: 240,
            overlay: Overlay::Hillshade,
        },
    )
    .unwrap();
    let mut inside = 0;
    for (x, _, p) in img.enumerate_pixels() {
        let east = 200.0 + f64::from(x) + 0.5;
        if p[3] > 0 {
            assert!((309.0..321.0).contains(&east), "hatching at {east} m");
            inside += 1;
        }
    }
    assert!(inside > 0, "the wall is hatched");
}

#[test]
fn hatching_stays_at_55_percent() {
    let hm = wall();
    let img = render_layer(
        MapLayer::TooSteepToWalk,
        &LayerData::new(&hm),
        &whole(&hm, 640),
    )
    .unwrap();
    let max = img.pixels().map(|p| p[3]).max().unwrap();
    assert!(max > 0 && max <= 141, "{max}");
}

#[test]
fn drawing_order_is_stable() {
    assert_eq!(
        MapLayer::ALL,
        &[
            MapLayer::TerrainChanges,
            MapLayer::TooSteepToWalk,
            MapLayer::Roads,
            MapLayer::Paths,
            MapLayer::Drivable,
        ]
    );
    assert_eq!(MapLayer::TooSteepToWalk.id(), "too_steep_to_walk");
}

#[test]
fn the_whole_map_can_be_asked_for() {
    let hm = wall();
    let img = render_layer(
        MapLayer::TooSteepToWalk,
        &LayerData::new(&hm),
        &whole(&hm, 320),
    )
    .unwrap();
    // 0.5 px per metre: the step is in the columns 155 to 160
    let (mut at_wall, mut elsewhere) = (0, 0);
    for (x, _, p) in img.enumerate_pixels() {
        if p[3] > 0 {
            if (154..161).contains(&x) {
                at_wall += 1;
            } else {
                elsewhere += 1;
            }
        }
    }
    assert!(at_wall > 0 && elsewhere == 0, "{at_wall} {elsewhere}");
}

#[test]
fn a_bad_request_is_an_error() {
    let hm = wall();
    let mut req = whole(&hm, 64);
    req.x1 = req.x0;
    assert_eq!(
        render_layer(MapLayer::TooSteepToWalk, &LayerData::new(&hm), &req).unwrap_err(),
        RenderError::Bounds
    );
}

// ---- the layers of the roads ----

use image::{Rgba, RgbaImage};
use naves_core::catalog::Family;
use naves_core::checks::{DriveState, PieceCheck};
use naves_core::features::{Feature, FeatureKind, Source};
use naves_core::render::{composite, fade};

fn road_feature(family: Family, points: &[(f64, f64)]) -> Feature {
    Feature {
        id: "r".to_string(),
        way_id: 1,
        source: Source::Osm,
        kind: FeatureKind::Road {
            family,
            drivable: true,
            bridge: false,
            tunnel: false,
        },
        points: points.to_vec(),
        nodes: (1..=i64::try_from(points.len()).unwrap()).collect(),
    }
}

fn path_feature(points: &[(f64, f64)]) -> Feature {
    Feature {
        id: "p".to_string(),
        way_id: 2,
        source: Source::Osm,
        kind: FeatureKind::Path { width_m: 1.6 },
        points: points.to_vec(),
        nodes: (1..=i64::try_from(points.len()).unwrap()).collect(),
    }
}

fn piece(state: DriveState, from: (f64, f64), to: (f64, f64)) -> PieceCheck {
    PieceCheck {
        slot: 0,
        family: Family::Asf3,
        start: from,
        end: to,
        width: 7.0,
        grade: 0.0,
        cross_fall: 0.0,
        grade_change: 0.0,
        miss_m: 0.0,
        off_ground_m: 0.0,
        side_slope: 0.0,
        state,
    }
}

fn layer_of(
    layer: MapLayer,
    hm: &Heightmap,
    features: &[Feature],
    pieces: &[PieceCheck],
    px: u32,
) -> RgbaImage {
    let data = LayerData {
        heightmap: hm,
        base: None,
        features,
        pieces,
    };
    render_layer(layer, &data, &whole(hm, px)).unwrap()
}

const INK: [u8; 3] = [0x1d, 0x29, 0x24];

fn rgb(p: Rgba<u8>) -> [u8; 3] {
    [p[0], p[1], p[2]]
}

#[test]
fn a_road_is_drawn_at_its_width() {
    let hm = map(64, |_, _| 100.0);
    let road = [road_feature(
        Family::Asf3,
        &[(320.0, 100.0), (320.0, 540.0)],
    )];
    // 1 px per metre: 7 m is 7 px of ink and a pixel of white casing on each side
    let img = layer_of(MapLayer::Roads, &hm, &road, &[], 640);
    let row = 320;
    assert_eq!(
        (rgb(*img.get_pixel(320, row)), img.get_pixel(320, row)[3]),
        (INK, 255)
    );
    assert_eq!(rgb(*img.get_pixel(322, row)), INK);
    assert_eq!(rgb(*img.get_pixel(324, row)), [255, 255, 255], "the casing");
    assert_eq!(img.get_pixel(329, row)[3], 0, "beyond the casing");
    assert_eq!(img.get_pixel(320, 30)[3], 0, "beyond the end of the road");
    // 0.25 px per metre: 7 m is 1.75 px
    let small = layer_of(MapLayer::Roads, &hm, &road, &[], 160);
    assert_eq!(rgb(*small.get_pixel(80, 80)), INK);
    // 0.1 px per metre: the road is a pixel and a half wide at least
    let tiny = layer_of(MapLayer::Roads, &hm, &road, &[], 64);
    let ink = (30..34)
        .filter(|x| rgb(*tiny.get_pixel(*x, 32)) == INK && tiny.get_pixel(*x, 32)[3] == 255)
        .count();
    assert!(ink >= 1, "{ink}");
}

#[test]
fn paths_are_dashed_and_brown() {
    let hm = map(64, |_, _| 100.0);
    let path = [path_feature(&[(100.0, 320.0), (540.0, 320.0)])];
    let img = layer_of(MapLayer::Paths, &hm, &path, &[], 640);
    let row = 320;
    let painted: Vec<bool> = (110..530).map(|x| img.get_pixel(x, row)[3] > 0).collect();
    assert!(
        painted.iter().any(|p| *p) && painted.iter().any(|p| !*p),
        "dashed"
    );
    let colour = img.pixels().find(|p| p[3] > 0).map(|p| rgb(*p)).unwrap();
    assert_eq!(colour, [0x8c, 0x6a, 0x45]);
}

#[test]
fn not_drivable_is_wider_than_fine() {
    let hm = map(64, |_, _| 100.0);
    let pieces = [
        piece(DriveState::Fine, (160.0, 100.0), (160.0, 500.0)),
        piece(DriveState::NotDrivable, (480.0, 100.0), (480.0, 500.0)),
    ];
    let img = layer_of(MapLayer::Drivable, &hm, &[], &pieces, 640);
    let width_at = |x: u32| {
        (x - 12..x + 12)
            .filter(|c| img.get_pixel(*c, 340)[3] > 0)
            .count()
    };
    assert!(
        width_at(480) >= 2 * width_at(160) - 1,
        "{} {}",
        width_at(480),
        width_at(160)
    );
    assert_eq!(rgb(*img.get_pixel(160, 340)), [0x2e, 0x6a, 0x4c]);
    assert_eq!(rgb(*img.get_pixel(480, 340)), [0xb0, 0x3a, 0x2e]);
}

#[test]
fn walk_only_is_not_coloured_as_a_problem() {
    let hm = map(64, |_, _| 100.0);
    let pieces = [piece(DriveState::WalkOnly, (160.0, 100.0), (160.0, 500.0))];
    let img = layer_of(MapLayer::Drivable, &hm, &[], &pieces, 640);
    assert_eq!(rgb(*img.get_pixel(160, 340)), [0x5a, 0x66, 0x60]);
    let steep = [piece(DriveState::Steep, (160.0, 100.0), (160.0, 500.0))];
    let amber = layer_of(MapLayer::Drivable, &hm, &[], &steep, 640);
    assert_eq!(rgb(*amber.get_pixel(160, 340)), [0xd0, 0x8a, 0x1e]);
}

#[test]
fn terrain_changes_tint_cut_and_fill_differently() {
    let base = map(64, |_, _| 100.0);
    // a cut of 4 m in the west, a fill of 6 m in the east, and ground that did not change
    let shaped = map(64, |col, _| {
        if col < 20.0 {
            96.0
        } else if col > 44.0 {
            106.0
        } else {
            100.0
        }
    });
    let data = LayerData {
        heightmap: &shaped,
        base: Some(&base),
        features: &[],
        pieces: &[],
    };
    let img = render_layer(MapLayer::TerrainChanges, &data, &whole(&shaped, 640)).unwrap();
    let cut = img.get_pixel(60, 320);
    let fill = img.get_pixel(580, 320);
    assert_eq!(rgb(*cut), [0x8c, 0x6a, 0x45]);
    assert_eq!(rgb(*fill), [0x3f, 0x6f, 0x9e]);
    assert_eq!(
        img.get_pixel(320, 320)[3],
        0,
        "unchanged ground draws nothing"
    );
    // 4 m of 5 is most of half, 6 m is the full half
    assert!(
        fill[3] > cut[3] && fill[3].abs_diff(127) <= 2,
        "{} {}",
        fill[3],
        cut[3]
    );
    // a change under a quarter of a metre is not shown
    let slight = map(64, |_, _| 100.2);
    let none = render_layer(
        MapLayer::TerrainChanges,
        &LayerData {
            heightmap: &slight,
            base: Some(&base),
            features: &[],
            pieces: &[],
        },
        &whole(&slight, 64),
    )
    .unwrap();
    assert!(none.pixels().all(|p| p[3] == 0));
}

#[test]
fn layers_of_the_roads_draw_nothing_without_roads() {
    let hm = map(64, |_, _| 100.0);
    for layer in [
        MapLayer::Roads,
        MapLayer::Paths,
        MapLayer::Drivable,
        MapLayer::TerrainChanges,
    ] {
        let img = layer_of(layer, &hm, &[], &[], 64);
        assert!(img.pixels().all(|p| p[3] == 0), "{layer:?}");
    }
}

#[test]
fn the_ids_of_the_new_layers_are_stable() {
    assert_eq!(MapLayer::Roads.id(), "roads");
    assert_eq!(MapLayer::Paths.id(), "paths");
    assert_eq!(MapLayer::Drivable.id(), "drivable");
    assert_eq!(MapLayer::TerrainChanges.id(), "terrain_changes");
}

#[test]
fn composite_keeps_what_is_below_transparent_pixels() {
    let below = RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 255]));
    let mut above = RgbaImage::new(4, 4);
    above.put_pixel(1, 1, Rgba([200, 100, 50, 255]));
    above.put_pixel(2, 2, Rgba([0, 0, 255, 128]));
    let out = composite(&below, &above);
    assert_eq!(out.get_pixel(0, 0), &Rgba([10, 20, 30, 255]));
    assert_eq!(out.get_pixel(1, 1), &Rgba([200, 100, 50, 255]));
    let mixed = out.get_pixel(2, 2);
    assert_eq!(mixed[3], 255);
    assert!(
        mixed[2] > 100 && mixed[2] < 160 && mixed[0] < 10,
        "{mixed:?}"
    );
}

#[test]
fn fade_changes_only_alpha() {
    let mut img = RgbaImage::new(2, 1);
    img.put_pixel(0, 0, Rgba([10, 20, 30, 200]));
    img.put_pixel(1, 0, Rgba([1, 2, 3, 0]));
    let faded = fade(&img, 0.5);
    assert_eq!(faded.get_pixel(0, 0), &Rgba([10, 20, 30, 100]));
    assert_eq!(faded.get_pixel(1, 0), &Rgba([1, 2, 3, 0]));
    assert_eq!(fade(&img, 1.0), img);
}

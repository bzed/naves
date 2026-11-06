// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Top-down rendering of a heightmap.

use naves_core::heightmap::Heightmap;
use naves_core::render::{Overlay, RenderError, RenderRequest, render_topdown};

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
    let img = render_topdown(&hm, &request(&hm, 64, Overlay::Height)).unwrap();
    let top = img.get_pixel(32, 0);
    let bottom = img.get_pixel(32, img.height() - 1);
    assert!(top[0] > bottom[0] + 100, "{top:?} vs {bottom:?}");
}

#[test]
fn east_is_right() {
    let hm = map(64, |col, _| col * 3.0);
    let img = render_topdown(&hm, &request(&hm, 64, Overlay::Height)).unwrap();
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
    let img = render_topdown(&hm, &req).unwrap();
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
    let img = render_topdown(&hm, &request(&hm, 64, Overlay::Slope)).unwrap();
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
    let lit = render_topdown(&faces_nw, &request(&faces_nw, 32, Overlay::Hillshade)).unwrap();
    let dark = render_topdown(&faces_se, &request(&faces_se, 32, Overlay::Hillshade)).unwrap();
    assert!(lit.get_pixel(16, 16)[0] > dark.get_pixel(16, 16)[0] + 50);
}

#[test]
fn flat_height_map_does_not_divide_by_zero() {
    let hm = map(16, |_, _| 5.0);
    let img = render_topdown(&hm, &request(&hm, 16, Overlay::Height)).unwrap();
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
                render_topdown(&hm, &bad),
                Err(RenderError::Bounds | RenderError::Width(_) | RenderError::Aspect)
            ),
            "{bad:?}"
        );
    }
}

#[test]
fn output_is_deterministic() {
    let hm = map(32, |col, row| col * row * 0.1);
    let a = render_topdown(&hm, &request(&hm, 48, Overlay::Slope)).unwrap();
    let b = render_topdown(&hm, &request(&hm, 48, Overlay::Slope)).unwrap();
    assert_eq!(a, b);
}

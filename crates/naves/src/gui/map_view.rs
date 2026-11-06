// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pan and zoom of the map view, in plain numbers so that the maths can be tested.
//!
//! Screen coordinates are pixels from the top-left corner of the view with y down; map coordinates are
//! metres from the south-west corner with y up.

/// Smallest and largest zoom: metres per screen pixel.
const MIN_MPP: f64 = 0.05;
const MAX_MPP: f64 = 2000.0;

/// Which part of the map is shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapView {
    /// Map position at the middle of the view.
    pub center: (f64, f64),
    /// Metres per screen pixel.
    pub metres_per_px: f64,
}

impl MapView {
    /// A view that shows the whole square map in a viewport of `width` × `height` pixels.
    pub fn fit(map_size_m: f64, width: f64, height: f64) -> Self {
        let side = width.min(height).max(1.0);
        Self {
            center: (map_size_m / 2.0, map_size_m / 2.0),
            metres_per_px: (map_size_m / side).clamp(MIN_MPP, MAX_MPP),
        }
    }

    /// The map position under the screen position (`sx`, `sy`).
    pub fn screen_to_map(&self, sx: f64, sy: f64, width: f64, height: f64) -> (f64, f64) {
        (
            self.center.0 + (sx - width / 2.0) * self.metres_per_px,
            self.center.1 - (sy - height / 2.0) * self.metres_per_px,
        )
    }

    /// Moves the map with the pointer by (`dx`, `dy`) screen pixels.
    pub fn pan_px(&mut self, dx: f64, dy: f64) {
        self.center.0 -= dx * self.metres_per_px;
        self.center.1 += dy * self.metres_per_px;
    }

    /// Zooms by `factor` (above 1 zooms out) keeping the map position under (`sx`, `sy`) in place.
    pub fn zoom_at(&mut self, sx: f64, sy: f64, width: f64, height: f64, factor: f64) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let anchor = self.screen_to_map(sx, sy, width, height);
        self.metres_per_px = (self.metres_per_px * factor).clamp(MIN_MPP, MAX_MPP);
        self.center = (
            anchor.0 - (sx - width / 2.0) * self.metres_per_px,
            anchor.1 + (sy - height / 2.0) * self.metres_per_px,
        );
    }

    /// The rectangle (`x0`, `y0`, `x1`, `y1`) in map metres that a view of `width` × `height` pixels shows.
    pub fn bounds(&self, width: f64, height: f64) -> (f64, f64, f64, f64) {
        let half_w = width / 2.0 * self.metres_per_px;
        let half_h = height / 2.0 * self.metres_per_px;
        (
            self.center.0 - half_w,
            self.center.1 - half_h,
            self.center.0 + half_w,
            self.center.1 + half_h,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn fit_shows_the_whole_map() {
        let v = MapView::fit(6400.0, 800.0, 600.0);
        let (x0, y0, x1, y1) = v.bounds(800.0, 600.0);
        assert!(
            x0 <= 0.0 && x1 >= 6400.0 && y0 <= 0.0 + 1000.0 && y1 >= 6400.0 - 1000.0,
            "{x0} {y0} {x1} {y1}"
        );
        assert!(close(v.center.0, 3200.0));
    }

    #[test]
    fn screen_centre_is_the_view_centre_and_north_is_up() {
        let v = MapView {
            center: (100.0, 200.0),
            metres_per_px: 2.0,
        };
        assert_eq!(v.screen_to_map(400.0, 300.0, 800.0, 600.0), (100.0, 200.0));
        let up = v.screen_to_map(400.0, 290.0, 800.0, 600.0);
        assert!(close(up.1, 220.0), "{up:?}");
    }

    #[test]
    fn pan_moves_the_map_with_the_pointer() {
        let mut v = MapView {
            center: (100.0, 100.0),
            metres_per_px: 2.0,
        };
        v.pan_px(10.0, 5.0); // drag right and down: the map follows, the view centre moves west and north
        assert_eq!(v.center, (80.0, 110.0));
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut v = MapView {
            center: (500.0, 500.0),
            metres_per_px: 4.0,
        };
        let (sx, sy, w, h) = (650.0, 120.0, 800.0, 600.0);
        let before = v.screen_to_map(sx, sy, w, h);
        v.zoom_at(sx, sy, w, h, 0.5);
        let after = v.screen_to_map(sx, sy, w, h);
        assert!(close(before.0, after.0) && close(before.1, after.1));
        assert!(close(v.metres_per_px, 2.0));
    }

    #[test]
    fn zoom_is_limited_and_ignores_bad_factors() {
        let mut v = MapView {
            center: (0.0, 0.0),
            metres_per_px: 1.0,
        };
        v.zoom_at(0.0, 0.0, 100.0, 100.0, 1e-12);
        assert!(close(v.metres_per_px, MIN_MPP));
        v.zoom_at(0.0, 0.0, 100.0, 100.0, 1e12);
        assert!(close(v.metres_per_px, MAX_MPP));
        let before = v;
        v.zoom_at(0.0, 0.0, 100.0, 100.0, f64::NAN);
        v.zoom_at(0.0, 0.0, 100.0, 100.0, -1.0);
        assert_eq!(v, before);
    }
}

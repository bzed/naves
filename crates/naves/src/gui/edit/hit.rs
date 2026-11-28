// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hit-testing the source ways in map metres.

use super::EditSession;
use super::properties::{origin_line, way_title};
use crate::gui::levels::Level;
use crate::gui::selection::Selection;
use crate::gui::viewport::WorldPoint;

/// How far from a line a click still hits it, in pixels.
pub const HIT_PX: f64 = 8.0;

/// A thing of the source layer a click can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A way, by raw id.
    Way(i64),
    /// A node of a way, by raw ids.
    Node {
        /// The node.
        node: i64,
        /// The way it was picked on.
        way: i64,
    },
}

/// How many metres a click may be off: [`HIT_PX`] pixels on a view that shows `view_span_m` metres in
/// `view_width_px` pixels, but at least half a metre.
#[must_use]
pub fn tolerance_m(view_span_m: f64, view_width_px: f64) -> f64 {
    let per_px = view_span_m / view_width_px.max(1.0);
    (HIT_PX * per_px).max(0.5)
}

/// The distance from `at` to the segment `a`-`b`.
fn segment_distance(at: WorldPoint, a: WorldPoint, b: WorldPoint) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length2 = dx * dx + dy * dy;
    let along = if length2 > 0.0 {
        (((at.x - a.x) * dx + (at.y - a.y) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (at.x - a.x - along * dx).hypot(at.y - a.y - along * dy)
}

/// The node or way at `at`: a node within `tol_m` wins over a way, the nearest wins, and a tie goes to the
/// higher raw id.
#[must_use]
pub fn hit(session: &EditSession, at: WorldPoint, tol_m: f64) -> Option<Target> {
    let mut node: Option<(f64, Target)> = None;
    let mut way: Option<(f64, Target)> = None;
    for w in session.projected() {
        let points = &w.points;
        for &(id, p) in points {
            let d = (p.x - at.x).hypot(p.y - at.y);
            if d <= tol_m && node.is_none_or(|(best, _)| d <= best) {
                node = Some((
                    d,
                    Target::Node {
                        node: id,
                        way: w.id,
                    },
                ));
            }
        }
        for pair in points.windows(2) {
            if let [(_, a), (_, b)] = pair {
                let d = segment_distance(at, *a, *b);
                if d <= tol_m && way.is_none_or(|(best, _)| d <= best) {
                    way = Some((d, Target::Way(w.id)));
                }
            }
        }
    }
    node.or(way).map(|(_, target)| target)
}

/// What a click at `at` picks of the source layer, if anything: the way or node with its title and where it
/// comes from. `tol_m` is [`tolerance_m`] of the view.
#[must_use]
pub fn pick_source(session: &EditSession, at: WorldPoint, tol_m: f64) -> Option<Selection> {
    let target = hit(session, at, tol_m)?;
    let (way_id, node) = match target {
        Target::Way(way) => (way, None),
        Target::Node { node, way } => (way, Some(node)),
    };
    let way = session.way(way_id)?;
    let mut rows = Vec::new();
    if let Some(node) = node {
        rows.push(("Node".to_string(), node.to_string()));
    }
    if let Some(name) = way.tags.get("name") {
        rows.push(("Name".to_string(), name.clone()));
    }
    if let Some(class) = way.tags.get("highway") {
        rows.push(("Highway".to_string(), class.clone()));
    }
    let origin = origin_line(session.origin(way_id), way_id);
    Some(Selection {
        level: Level::Roads,
        title: way_title(&way.tags),
        rows,
        origin: Some(origin),
        at,
        span_m: None,
        target: Some(target),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::edit::fixture::session;

    fn at(x: f64, y: f64) -> WorldPoint {
        WorldPoint { x, y }
    }

    #[test]
    fn a_node_wins_over_its_way() {
        let s = session(&[(7, &[(500.0, 500.0), (550.0, 500.0), (600.0, 500.0)])]);
        assert_eq!(
            hit(&s, at(551.0, 501.0), 4.0),
            Some(Target::Node { node: 701, way: 7 })
        );
        assert_eq!(hit(&s, at(525.0, 501.0), 4.0), Some(Target::Way(7)));
        assert_eq!(hit(&s, at(525.0, 510.0), 4.0), None);
    }

    #[test]
    fn the_tolerance_follows_the_zoom() {
        assert!((tolerance_m(1000.0, 1000.0) - 8.0).abs() < 1e-12);
        assert!((tolerance_m(10.0, 1000.0) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn the_nearest_wins_and_a_tie_goes_to_the_higher_id() {
        let s = session(&[
            (3, &[(500.0, 500.0), (600.0, 500.0)]),
            (9, &[(500.0, 500.0), (600.0, 500.0)]),
            (5, &[(500.0, 504.0), (600.0, 504.0)]),
        ]);
        assert_eq!(hit(&s, at(550.0, 501.0), 4.0), Some(Target::Way(9)));
        assert_eq!(hit(&s, at(550.0, 503.0), 4.0), Some(Target::Way(5)));
        // the same node of two ways: the later way
        assert_eq!(
            hit(&s, at(500.0, 500.0), 1.0),
            Some(Target::Node { node: 900, way: 9 })
        );
    }
}

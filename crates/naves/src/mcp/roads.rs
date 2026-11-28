// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The road tools: the JSON of a way, the rectangle test and the arguments of the tools. Coordinates are map
//! metres from the south-west corner; ids are strings in the edit-file notation (`"812"`, `"u3"`).

use std::collections::BTreeSet;

use naves_core::edits::{ElementId, USER_ID_BASE};
use naves_core::osm::OsmWay;
use naves_export::generate::Source;
use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};

/// Arguments of `roads_list`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RectParams {
    /// West edge in map metres.
    pub x0: f64,
    /// South edge in map metres.
    pub y0: f64,
    /// East edge in map metres.
    pub x1: f64,
    /// North edge in map metres.
    pub y1: f64,
    /// At most this many ways (default and maximum 1000).
    pub limit: Option<u32>,
}

/// Rounds to 0.01 m.
fn centimetres(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The JSON of a way: `{"id", "origin": "osm" | "edited" | "new", "tags", "points": [[x, y], ...]}`.
/// `edited` holds the raw ids of the ways with an edit and `moved` those of the nodes that were moved. Points
/// are map metres rounded to 0.01 m; a point that cannot be projected is left out.
#[must_use]
pub fn way_json(
    source: &Source,
    way: &OsmWay,
    edited: &BTreeSet<i64>,
    moved: &BTreeSet<i64>,
) -> Value {
    let origin = if way.id >= USER_ID_BASE {
        "new"
    } else if edited.contains(&way.id) || way.nodes.iter().any(|n| moved.contains(n)) {
        "edited"
    } else {
        "osm"
    };
    let id = ElementId::from_raw(way.id).map_or_else(|| way.id.to_string(), |id| id.to_string());
    let points: Vec<Value> = way
        .points
        .iter()
        .filter_map(|&(lat, lon)| {
            let (x, y) = source.frame.to_map(lat, lon).ok()?;
            (x.is_finite() && y.is_finite()).then(|| json!([centimetres(x), centimetres(y)]))
        })
        .collect();
    json!({"id": id, "origin": origin, "tags": way.tags, "points": points})
}

/// Whether the closed rectangle `(x0, y0, x1, y1)` holds a point of the line or a segment of it crosses it.
#[must_use]
pub fn touches(points: &[(f64, f64)], rect: (f64, f64, f64, f64)) -> bool {
    let (x0, y0, x1, y1) = rect;
    let inside = |&(x, y): &(f64, f64)| x >= x0 && x <= x1 && y >= y0 && y <= y1;
    if points.iter().any(inside) {
        return true;
    }
    points.windows(2).any(|pair| match pair {
        [a, b] => crosses(*a, *b, rect),
        _ => false,
    })
}

/// Liang-Barsky clipping of the segment `a`-`b` against the rectangle.
fn crosses(a: (f64, f64), b: (f64, f64), rect: (f64, f64, f64, f64)) -> bool {
    let (x0, y0, x1, y1) = rect;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-dx, a.0 - x0),
        (dx, x1 - a.0),
        (-dy, a.1 - y0),
        (dy, y1 - a.1),
    ] {
        if p.abs() < f64::EPSILON {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::touches;

    const RECT: (f64, f64, f64, f64) = (0.0, 0.0, 10.0, 10.0);

    #[test]
    fn a_point_inside_or_on_the_edge_touches() {
        assert!(touches(&[(5.0, 5.0)], RECT));
        assert!(touches(&[(10.0, 10.0)], RECT));
    }

    #[test]
    fn a_segment_through_the_rectangle_touches_without_a_point_in_it() {
        assert!(touches(&[(-5.0, 5.0), (15.0, 5.0)], RECT));
        assert!(touches(&[(-5.0, 15.0), (15.0, -5.0)], RECT));
    }

    #[test]
    fn a_line_beside_or_past_the_rectangle_does_not() {
        assert!(!touches(&[(-5.0, 11.0), (15.0, 11.0)], RECT));
        assert!(!touches(&[(-5.0, 15.0), (-1.0, 11.0)], RECT));
        assert!(!touches(&[(11.0, 0.0), (11.0, 10.0)], RECT));
        assert!(!touches(&[], RECT));
    }
}

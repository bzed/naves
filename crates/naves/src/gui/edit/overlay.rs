// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The shapes of the editing overlay, in map metres.

use super::hit::{Target, road_points};
use super::{EditSession, Origin};
use crate::gui::viewport::WorldPoint;

/// How a line is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    /// A way as read from OpenStreetMap.
    Source,
    /// A way that was changed or drawn.
    Edited,
    /// The selected way.
    Selected,
    /// The line being drawn.
    Drawing,
}

/// A shape of the overlay.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayShape {
    /// A line.
    Line {
        /// Its points.
        points: Vec<WorldPoint>,
        /// How it looks.
        style: LineStyle,
    },
    /// A node handle.
    Handle(WorldPoint),
    /// A tick across the end of an edited way.
    Tick {
        /// Where.
        at: WorldPoint,
        /// The direction of the line there, in radians from east, counter-clockwise.
        heading: f64,
    },
}

/// The shapes for the ways of the session, the selected one with its handles, and the line being drawn. The
/// order is the paint order: source lines at the bottom, then edited ones, the selected one, the line being drawn,
/// and the ticks and handles on top.
#[must_use]
pub fn overlay(
    session: &EditSession,
    selection: Option<&Target>,
    drawing: &[WorldPoint],
) -> Vec<OverlayShape> {
    let selected = selection.map(|target| match *target {
        Target::Way(way) | Target::Node { way, .. } => way,
    });
    let (mut source, mut edited, mut chosen) = (Vec::new(), Vec::new(), Vec::new());
    let (mut ticks, mut handles) = (Vec::new(), Vec::new());
    for way in &session.source.ways.ways {
        let points: Vec<WorldPoint> = road_points(session, way).into_iter().map(|p| p.1).collect();
        if points.len() < 2 {
            continue;
        }
        let is_selected = selected == Some(way.id);
        let is_edited = session.origin(way.id) != Origin::Osm;
        if is_edited {
            ticks.extend(end_ticks(&points));
        }
        if is_selected {
            handles.extend(points.iter().copied().map(OverlayShape::Handle));
        }
        let (list, style) = if is_selected {
            (&mut chosen, LineStyle::Selected)
        } else if is_edited {
            (&mut edited, LineStyle::Edited)
        } else {
            (&mut source, LineStyle::Source)
        };
        list.push(OverlayShape::Line { points, style });
    }
    let mut shapes = source;
    shapes.append(&mut edited);
    shapes.append(&mut chosen);
    if !drawing.is_empty() {
        shapes.push(OverlayShape::Line {
            points: drawing.to_vec(),
            style: LineStyle::Drawing,
        });
        handles.extend(drawing.iter().copied().map(OverlayShape::Handle));
    }
    shapes.append(&mut ticks);
    shapes.append(&mut handles);
    shapes
}

/// The ticks at both ends of a line, along the direction of the line there.
fn end_ticks(points: &[WorldPoint]) -> Vec<OverlayShape> {
    let heading = |a: WorldPoint, b: WorldPoint| (b.y - a.y).atan2(b.x - a.x);
    match (
        points.first(),
        points.get(1),
        points.last(),
        points.iter().rev().nth(1),
    ) {
        (Some(&first), Some(&second), Some(&last), Some(&before)) => vec![
            OverlayShape::Tick {
                at: first,
                heading: heading(first, second),
            },
            OverlayShape::Tick {
                at: last,
                heading: heading(before, last),
            },
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use naves_core::edits::By;

    use super::*;
    use crate::gui::edit::fixture::session;

    fn count(shapes: &[OverlayShape], style: LineStyle) -> usize {
        shapes
            .iter()
            .filter(|s| matches!(s, OverlayShape::Line { style: st, .. } if *st == style))
            .count()
    }

    fn ticks(shapes: &[OverlayShape]) -> usize {
        shapes
            .iter()
            .filter(|s| matches!(s, OverlayShape::Tick { .. }))
            .count()
    }

    #[test]
    fn edited_and_drawn_ways_get_ticks() {
        let mut s = session(&[
            (1, &[(500.0, 500.0), (600.0, 500.0)]),
            (2, &[(500.0, 600.0), (600.0, 600.0)]),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let original = s.original(2).unwrap().clone();
        s.apply(dir.path(), |edits, source| {
            let mut tags = original.tags.clone();
            tags.insert("surface".into(), "asphalt".into());
            let nodes = original
                .points
                .iter()
                .map(|&(lat, lon)| edits.new_node(lat, lon, By::User))
                .collect::<Result<Vec<_>, _>>()?;
            edits.modify_way(&original, tags, nodes, By::User)?;
            let (lat, lon) = source.frame.to_geo(700.0, 700.0).unwrap();
            let a = edits.new_node(lat, lon, By::User)?;
            let (lat, lon) = source.frame.to_geo(800.0, 700.0).unwrap();
            let b = edits.new_node(lat, lon, By::User)?;
            let tags = [("highway".to_string(), "track".to_string())].into();
            edits.add_way(tags, vec![a, b], By::User)?;
            Ok(())
        })
        .unwrap();
        let shapes = overlay(&s, None, &[]);
        assert_eq!(count(&shapes, LineStyle::Source), 1);
        assert_eq!(count(&shapes, LineStyle::Edited), 2);
        assert_eq!(ticks(&shapes), 4);
    }

    #[test]
    fn the_selected_way_has_handles() {
        let s = session(&[
            (1, &[(500.0, 500.0), (600.0, 500.0)]),
            (2, &[(500.0, 600.0), (550.0, 650.0), (600.0, 600.0)]),
        ]);
        let shapes = overlay(&s, Some(&Target::Way(2)), &[]);
        assert_eq!(count(&shapes, LineStyle::Selected), 1);
        assert_eq!(count(&shapes, LineStyle::Source), 1);
        let handles = shapes
            .iter()
            .filter(|s| matches!(s, OverlayShape::Handle(_)))
            .count();
        assert_eq!(handles, 3);
        let node = Target::Node { node: 200, way: 2 };
        assert_eq!(
            count(&overlay(&s, Some(&node), &[]), LineStyle::Selected),
            1
        );
    }

    #[test]
    fn the_line_being_drawn_has_a_handle_per_point() {
        let s = session(&[]);
        let drawing = [WorldPoint { x: 1.0, y: 1.0 }, WorldPoint { x: 5.0, y: 1.0 }];
        let shapes = overlay(&s, None, &drawing);
        assert_eq!(count(&shapes, LineStyle::Drawing), 1);
        assert_eq!(shapes.len(), 3);
    }
}

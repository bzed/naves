// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The decks of the placed bridges as the terrain shaping sees them.
//!
//! A [`BridgeSpan`] is one placed part of a bridge: a bridge model between its ports, or one step of a panel
//! grid, with the height of its deck surface at both ends and its [`PartShape`]. The terrain under it is shaped
//! from this and nothing else (see [`crate::conform::conform_bridges`]), so the deck's outline and height are
//! those of the placed network (a grid lengthened over a road, a model slid or grown, a grid that follows a
//! bend), not those of the OpenStreetMap line.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::bridgekind::panel_slab;
use crate::catalog::{PartShape, PieceCatalog};
use crate::profile::RoadStrip;
use crate::roadnet::RoadGraph;
use crate::roadplace::{PlacedKind, PlacedNetwork};

/// Ground points less than this from an end of a span are the end.
const EDGE_M: f64 = 1e-9;

/// One placed part of a bridge.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeSpan {
    /// The id of the bridge stretch it belongs to.
    pub stretch: String,
    /// Where the part's first port lies (`x` east, `y` north, in map metres) and the height of the deck surface
    /// there.
    pub first: (f64, f64, f64),
    /// The second port, and the deck height there.
    pub second: (f64, f64, f64),
    /// What is below the deck, measured from the first port; `across` is positive to the left of the way from
    /// the first port to the second.
    pub shape: PartShape,
    /// The strips of the road the bridge is part of, up to the next junction at each end (see
    /// [`bridge_spans`]); the shaping adds the strips that have a deck joint at one of its ports. None of them is
    /// another road under the deck.
    pub own_strips: Vec<usize>,
}

impl BridgeSpan {
    /// The distance between the ports on the map, in metres.
    #[must_use]
    pub fn length(&self) -> f64 {
        (self.second.0 - self.first.0).hypot(self.second.1 - self.first.1)
    }

    /// The unit vector from the first port to the second and the one to its left; `None` for a part that has no
    /// direction (the ports are the same point or not numbers).
    #[must_use]
    pub fn axes(&self) -> Option<((f64, f64), (f64, f64))> {
        let (dx, dy) = (self.second.0 - self.first.0, self.second.1 - self.first.1);
        let length = dx.hypot(dy);
        (length.is_finite() && length > EDGE_M).then(|| {
            let along = (dx / length, dy / length);
            (along, (-along.1, along.0))
        })
    }

    /// The height of the deck surface `along` metres from the first port (outside the span: at its end).
    #[must_use]
    pub fn deck_at(&self, along: f64) -> f64 {
        let length = self.length();
        if length > EDGE_M && along.is_finite() {
            let t = (along / length).clamp(0.0, 1.0);
            self.first.2 + (self.second.2 - self.first.2) * t
        } else {
            self.first.2
        }
    }

    /// `(along, across)` of the map point `p`: metres from the first port along the span and to the left of it.
    #[must_use]
    pub fn locate(&self, p: (f64, f64)) -> Option<(f64, f64)> {
        let (along, left) = self.axes()?;
        let (dx, dy) = (p.0 - self.first.0, p.1 - self.first.1);
        Some((dx * along.0 + dy * along.1, dx * left.0 + dy * left.1))
    }

    /// The map point `along` metres from the first port and `across` to the left of the span.
    #[must_use]
    pub fn point(&self, along: f64, across: f64) -> Option<(f64, f64)> {
        let (a, l) = self.axes()?;
        Some((
            self.first.0 + along * a.0 + across * l.0,
            self.first.1 + along * a.1 + across * l.1,
        ))
    }

    /// What hangs under the deck `along` metres from the first port, without what stands: the slice's hang.
    #[must_use]
    pub fn hang_at(&self, along: f64) -> f64 {
        let n = self.shape.underside.depth.len();
        if n == 0 {
            return 0.0;
        }
        let slice = if along.is_finite() && along > 0.0 {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a slice index, clamped to the slices below"
            )]
            let i = (along / crate::catalog::SLICE_M).floor().min(1.0e9) as usize;
            i.min(n - 1)
        } else {
            0
        };
        self.shape
            .underside
            .hang
            .get(slice)
            .or_else(|| self.shape.underside.depth.get(slice))
            .copied()
            .unwrap_or(0.0)
            .max(0.0)
    }

    /// How far the part reaches from its axis at `along`, to either side: what hangs or stands there or the
    /// deck, whichever is wider.
    #[must_use]
    pub fn half_width_at(&self, along: f64) -> f64 {
        let n = self.shape.underside.half_width.len();
        if n == 0 {
            return self.shape.deck_half_width.max(0.0);
        }
        let slice = if along.is_finite() && along > 0.0 {
            // a slice index from a clamped, non-negative length
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a slice index, clamped to the slices below"
            )]
            let i = (along / crate::catalog::SLICE_M).floor().min(1.0e9) as usize;
            i.min(n - 1)
        } else {
            0
        };
        self.shape
            .underside
            .half_width
            .get(slice)
            .copied()
            .unwrap_or(0.0)
            .max(self.shape.deck_half_width)
            .max(0.0)
    }
}

/// `(x, z)` of a model's frame turned by `theta` into the map (the placement's `rotate`).
fn turned(x: f64, z: f64, theta: f64) -> (f64, f64) {
    let (s, c) = theta.sin_cos();
    (x * c + z * s, -x * s + z * c)
}

/// The ids of the stretches `slot` is made of: a junction piece carries two, a part split from a stretch is the
/// stretch.
fn stretch_ids(slot: &str) -> Vec<String> {
    slot.split('|')
        .map(|p| p.split('/').next().unwrap_or(p).to_string())
        .collect()
}

/// The node ids in a stretch id (`<way>:<start>-<end>`), for a stretch that the graph does not have; the ids of the
/// map's edge (negative) are left out. A negative id is written with its own minus sign: `-5--6` is the ids -5 and -6.
fn node_ids(id: &str) -> Vec<String> {
    let Some((_, nodes)) = id.split_once(':') else {
        return Vec::new();
    };
    let (start_is_edge, rest) = match nodes.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, nodes),
    };
    let Some((start, end)) = rest.split_once('-') else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if !start_is_edge && !start.is_empty() {
        out.push(start.to_string());
    }
    if !end.is_empty() && !end.starts_with('-') {
        out.push(end.to_string());
    }
    out
}

/// The nodes of every stretch of the placed network, by the id of the stretch a slot names (without the `/...` of
/// a piece cut from it): the nodes of the graph's stretch as it is, which is not what its id spells after a
/// clean-up moved an end. A stretch that was cut into pieces has the ends of the pieces that no other piece of it
/// shares. A stretch the graph does not have is read from its id. The ids of the map's edge (negative) are left
/// out.
fn nodes_of_stretches(graph: &RoadGraph) -> BTreeMap<String, Vec<String>> {
    let mut count: BTreeMap<String, BTreeMap<i64, usize>> = BTreeMap::new();
    for s in &graph.stretches {
        let base = s.id.split('/').next().unwrap_or(&s.id).to_string();
        let nodes = count.entry(base).or_default();
        for node in [s.start, s.end] {
            *nodes.entry(node).or_default() += 1;
        }
    }
    count
        .into_iter()
        .map(|(base, nodes)| {
            let pieces = graph
                .stretches
                .iter()
                .filter(|s| s.id.split('/').next() == Some(base.as_str()))
                .count();
            let ends = nodes
                .into_iter()
                // the ends of one piece are all there is; of several, the nodes that only one piece has
                .filter(|&(node, n)| node > 0 && (pieces == 1 || n == 1))
                .map(|(node, _)| node.to_string())
                .collect();
            (base, ends)
        })
        .collect()
}

/// The stretches of the road `bridge` is part of: itself and, through every node where exactly two stretches
/// meet, the stretches beyond it, up to the next junction at each end.
fn chain_of(
    bridge: &str,
    nodes_of: &dyn Fn(&str) -> Vec<String>,
    at_node: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut chain = BTreeSet::from([bridge.to_string()]);
    let mut queue = VecDeque::from([bridge.to_string()]);
    while let Some(id) = queue.pop_front() {
        for node in nodes_of(&id) {
            let here = at_node.get(&node);
            if let Some(here) = here.filter(|h| h.len() == 2) {
                for other in here {
                    if chain.insert(other.clone()) {
                        queue.push_back(other.clone());
                    }
                }
            }
        }
    }
    chain
}

/// The placed parts of every bridge in `strips`, in the order of the strips and of the decks in them, from the
/// heights the height solve gave their joints. A deck whose model is not in `catalog` (and is not its panel)
/// is left out.
#[must_use]
pub fn bridge_spans(
    net: &PlacedNetwork,
    strips: &[RoadStrip],
    catalog: &PieceCatalog,
    graph: &RoadGraph,
) -> Vec<BridgeSpan> {
    let slab = panel_slab(catalog);
    let known = nodes_of_stretches(graph);
    let nodes_of = |id: &str| {
        let base = id.split('/').next().unwrap_or(id);
        known.get(base).cloned().unwrap_or_else(|| node_ids(base))
    };
    let mut at_node: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for road in &net.roads {
        for id in stretch_ids(&road.slot.stretch) {
            for node in nodes_of(&id) {
                at_node.entry(node).or_default().insert(id.clone());
            }
        }
    }
    let mut own: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut out = Vec::new();
    for strip in strips {
        for (segment, pair) in strip.joints.windows(2).enumerate() {
            let [from, to] = pair else { continue };
            if !strip.bridge.get(segment).copied().unwrap_or(false) {
                continue;
            }
            let Some(road) = strip.slots.get(segment).and_then(|i| net.roads.get(*i)) else {
                continue;
            };
            if road.kind != PlacedKind::Bridge {
                continue;
            }
            // a step of a lengthened panel grid starts where the placed panel does: the strip's joint before the first
            // step is the end of the junction piece that the grid lies over, inside the deck (the height is the joint's)
            let (from, to) = if slab.is_some_and(|s| s.model == road.model) {
                (
                    (road.start.x, road.start.y, from.2),
                    (road.end.x, road.end.y, to.2),
                )
            } else {
                (*from, *to)
            };
            let chord = (to.0 - from.0).hypot(to.1 - from.1);
            let (shape, reversed) = if let Some(model) =
                catalog.bridges().iter().find(|m| m.model == road.model)
            {
                // the model's first port is at the road's start, or, where it lies the other way round the
                // stretch, at its end
                let (dx, dz) = turned(model.start.x, model.start.z, road.origin.heading);
                let port = (road.origin.x + dx, road.origin.y + dz);
                let near = |p: (f64, f64)| (port.0 - p.0).hypot(port.1 - p.1);
                let reversed = near((road.end.x, road.end.y)) < near((road.start.x, road.start.y));
                (model.shape(), reversed)
            } else if let Some(slab) = slab.filter(|s| s.model == road.model) {
                (
                    PartShape::level(chord, slab.thickness * road.scale, road.width / 2.0),
                    false,
                )
            } else {
                continue;
            };
            let (first, second) = if reversed { (to, from) } else { (from, to) };
            let own_strips = own
                .entry(road.slot.stretch.clone())
                .or_insert_with(|| {
                    let chain = chain_of(&road.slot.stretch, &nodes_of, &at_node);
                    strips
                        .iter()
                        .enumerate()
                        .filter(|(_, strip)| {
                            strip
                                .slots
                                .iter()
                                .filter_map(|i| net.roads.get(*i))
                                .any(|r| {
                                    stretch_ids(&r.slot.stretch)
                                        .iter()
                                        .any(|id| chain.contains(id))
                                })
                        })
                        .map(|(i, _)| i)
                        .collect()
                })
                .clone();
            out.push(BridgeSpan {
                stretch: road.slot.stretch.clone(),
                first,
                second,
                shape,
                own_strips,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::node_ids;

    #[test]
    fn the_nodes_of_an_id_leave_out_the_ids_of_the_maps_edge() {
        assert_eq!(node_ids("7:12-34"), ["12", "34"]);
        assert_eq!(node_ids("7:-12-34"), ["34"]);
        assert_eq!(node_ids("7:12--34"), ["12"]);
        assert_eq!(node_ids("7:-12--34"), Vec::<String>::new());
        assert_eq!(node_ids("no colon"), Vec::<String>::new());
    }
}

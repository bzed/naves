// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Generation: everything the build decides before a file is written.
//!
//! DEM, base heightmap, the OSM ways, features, the road graph, the catalog of the game's road pieces,
//! the placement, height profiles, the terrain shaped to fit them, the objects and the user's edits, the
//! ground on the shaped terrain, the paths and the road bed painted on it, the checks and the violations.
//! Nothing here touches the project folder or runs a tool other than armake2 for the catalog, so the editor
//! can run it too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use naves_core::catalog::{BridgeModel, PieceCatalog};
use naves_core::checks::{DriveState, PieceCheck, check_roads_on_terrain, check_walkable};
use naves_core::conform::{Conformed, LevelPatch, conform};
use naves_core::dem::Dem;
use naves_core::dips::{Dip, DipAction, decide, find_dips};
use naves_core::features::{Feature, FeatureKind, road_features, water_lines};
use naves_core::heightmap::{Heightmap, HeightmapError, MapGeo, build_heightmap};
use naves_core::junctions::{JunctionKind, Junctions, find_junctions};
use naves_core::objects::{
    GeneratedObject, ObjectId, Overrides, PlacedObject, apply_overrides, bridge_object_height,
    road_objects,
};
use naves_core::osm::{LandPolygon, OsmWays, read_ways};
use naves_core::profile::{Profiles, solve_profiles};
use naves_core::project::Project;
use naves_core::roadnet::{RoadGraph, build_graph, lost_bridges, widen_bridges, with_bridge_spans};
use naves_core::roadplace::{PlacedKind, PlacedNetwork, bridge_for, place_network};
use naves_core::sideslope::side_slopes;
use naves_core::surface::{LoadedSources, SurfaceClass, SurfaceError, SurfaceGrid, build_surfaces};
use naves_core::validate::{Violation, check_roads};
use serde::Serialize;

use crate::build::BuildError;
use crate::catalog_read::road_catalog;
use crate::tools::Tools;

/// Share of the map that the land cover may leave undecided before the build says so.
const UNCOVERED_WARN_SHARE: f64 = 0.01;
/// Extra width of the ground painted under a road, so that grass does not grow through it.
const ROAD_BED_MARGIN_M: f64 = 2.0;
/// Margin around the map in which OSM ways are read, in metres.
const WAY_MARGIN_M: f64 = 100.0;

/// What the road network came to.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct RoadStats {
    /// Kilometres of road per family, in map metres.
    pub km_by_family: BTreeMap<String, f64>,
    /// How many stretches the roads were cut into.
    pub stretches: usize,
    /// Stretches that meet what they have to meet.
    pub closed: usize,
    /// Stretches that miss by a tolerable amount; not told apart from `closed` (the placement reports only
    /// the stretches it could not close).
    pub tolerated: usize,
    /// Stretches that miss by more than the tolerance.
    pub open: usize,
    /// Median distance of the placed pieces from the OSM line.
    pub median_deviation_m: f64,
    /// The steepest grade of any piece.
    pub max_grade: f64,
    /// The deepest cut into the terrain.
    pub max_cut_m: f32,
    /// The highest fill.
    pub max_fill_m: f32,
    /// How many road objects were placed.
    pub pieces: usize,
    /// Length of road that vehicles cannot drive, in map metres.
    pub not_drivable_m: f64,
    /// The largest distance of any piece above or below the terrain.
    pub largest_miss_m: f64,
    /// Length of paths that are too steep to walk, in map metres.
    pub paths_too_steep_m: f64,
}

/// Everything the build needs, and the editor shows.
#[derive(Debug, Clone)]
pub struct Generated {
    /// The terrain from the DEM.
    pub base: Heightmap,
    /// The terrain shaped to fit the roads (the base if there are none).
    pub heightmap: Heightmap,
    /// The ground on the shaped terrain, with the road beds and paths painted on it.
    pub surfaces: SurfaceGrid,
    /// The objects of the world, with the user's edits applied, in the order of their ids.
    pub objects: Vec<PlacedObject>,
    /// The height at which each object stands, in the order of `objects`.
    pub heights: Vec<f32>,
    /// The roads and paths in map coordinates.
    pub features: Vec<Feature>,
    /// The measurements of every road piece.
    pub pieces: Vec<PieceCheck>,
    /// Everything that is wrong, sorted by rule.
    pub violations: Vec<Violation>,
    /// The numbers.
    pub stats: RoadStats,
    /// Things worth knowing about the ground and the roads that are not violations.
    pub warnings: Vec<String>,
    /// The land cover was used.
    pub used_land_cover: bool,
    /// OpenStreetMap was used.
    pub used_osm: bool,
}

/// The roads of a project: nothing when it has none.
struct Roads {
    net: PlacedNetwork,
    graph: RoadGraph,
    profiles: Profiles,
    junctions: Junctions,
    conformed: Option<Conformed>,
    catalog: Option<PieceCatalog>,
    /// The dips of the final placement and what is done about each.
    dips: Vec<(Dip, DipAction)>,
}

/// The roads and streams read from OpenStreetMap.
struct Read {
    features: Vec<Feature>,
    /// Streams, rivers and canals in map metres, for the dips.
    water: Vec<Vec<(f64, f64)>>,
}

fn read_features(
    project: &Project,
    project_dir: &Path,
    frame: &MapGeo,
    progress: &dyn Fn(&str),
) -> Result<Read, BuildError> {
    if project.sources.osm.is_empty() {
        return Ok(Read {
            features: Vec::new(),
            water: Vec::new(),
        });
    }
    progress("Reading the roads");
    let window = frame.window(WAY_MARGIN_M).map_err(HeightmapError::from)?;
    let files: Vec<PathBuf> = project
        .sources
        .osm
        .iter()
        .map(|p| project_dir.join(p))
        .collect();
    let ways: OsmWays = read_ways(&files, window).map_err(SurfaceError::from)?;
    Ok(Read {
        features: road_features(&ways, frame, project.map_size_m()),
        water: water_lines(&ways, frame, project.map_size_m()),
    })
}

/// One placement of a graph: the pieces, the junction heights and the profiles of the roads.
pub struct Placed {
    /// The placed objects.
    pub net: PlacedNetwork,
    /// The junctions and their heights.
    pub junctions: Junctions,
    /// The side slope of the original ground at each road of `net`.
    pub side: Vec<f64>,
    /// The height profiles.
    pub profiles: Profiles,
}

/// Places `graph` on `base` and solves the heights of the roads.
///
/// # Errors
/// A stretch that cannot be fitted.
pub fn place_once(
    graph: &RoadGraph,
    catalog: &PieceCatalog,
    base: &Heightmap,
) -> Result<Placed, BuildError> {
    let net = place_network(graph, catalog)?;
    let junctions = find_junctions(&net, graph, base);
    let side = side_slopes(&net, base);
    let profiles = solve_profiles(&net, base, &junctions, &side);
    Ok(Placed {
        net,
        junctions,
        side,
        profiles,
    })
}

/// The graph of `features` and the same with every OpenStreetMap bridge grown to the length of the bridge
/// model it gets (see [`widen_bridges`]).
#[must_use]
pub fn widened_graph(features: &[Feature], catalog: &PieceCatalog) -> (RoadGraph, RoadGraph) {
    let built = build_graph(features);
    let widened = widen_bridges(&built, &|s| model_length(catalog, s.family).unwrap_or(0.0));
    (built, widened)
}

/// The scaled length of the bridge model a road of `family` gets.
fn model_length(catalog: &PieceCatalog, family: naves_core::catalog::Family) -> Option<f64> {
    bridge_for(catalog, family).map(|(model, scale, _)| model.length * scale)
}

/// The roads as they are built: the final graph, its placement and what was decided about the dips.
pub struct RoadPlan {
    /// The graph the roads were placed from: with the bridge spans over dips cut in.
    pub graph: RoadGraph,
    /// The final placement.
    pub placed: Placed,
    /// The dips of the first placement and what was decided about each.
    pub first_round: Vec<(Dip, DipAction)>,
    /// The dips of the final placement and what is done about each: never `Bridge`, a bridge is cut in once.
    pub dips: Vec<(Dip, DipAction)>,
    /// Whether the roads were placed a second time with bridges over dips.
    pub placed_twice: bool,
}

/// Places the roads and decides what to do about each dip: the order is fixed. Place the graph with the
/// bridges of OpenStreetMap, find the dips, and where a bridge fits cut it into the graph and place again;
/// there are no further rounds, the dips that are left are only told apart into fill and report.
///
/// `water` are the streams in map metres and `lakes` the areas of the project, which tell whether a dip lies
/// over water.
///
/// # Errors
/// A stretch that cannot be fitted.
pub fn plan_roads(
    features: &[Feature],
    catalog: &PieceCatalog,
    base: &Heightmap,
    water: &[Vec<(f64, f64)>],
    lakes: &[LandPolygon],
    frame: &MapGeo,
    progress: &dyn Fn(&str),
) -> Result<RoadPlan, BuildError> {
    let (built, widened) = widened_graph(features, catalog);
    let first = place_once(&widened, catalog, base)?;
    let dips = |graph: &RoadGraph, placed: &Placed, bridges: bool| {
        find_dips(
            graph,
            &placed.net,
            &placed.profiles,
            base,
            water,
            lakes,
            frame,
        )
        .into_iter()
        .map(|dip| {
            let length = bridges
                .then(|| graph.stretches.get(dip.stretch))
                .flatten()
                .and_then(|s| model_length(catalog, s.family));
            let (action, span) = decide(&dip, graph, length);
            (dip, action, span)
        })
        .collect::<Vec<_>>()
    };
    let decided = dips(&widened, &first, true);
    let spans: Vec<_> = decided.iter().filter_map(|d| d.2.clone()).collect();
    let first_round: Vec<(Dip, DipAction)> = decided.into_iter().map(|d| (d.0, d.1)).collect();
    let (graph, mut placed, dips, placed_twice) = if spans.is_empty() {
        (widened.clone(), first, first_round.clone(), false)
    } else {
        progress("Placing the roads again with bridges over the dips");
        let graph = with_bridge_spans(&widened, &spans);
        let placed = place_once(&graph, catalog, base)?;
        // the dips that are left cannot be bridged any more
        let left = dips(&graph, &placed, false)
            .into_iter()
            .map(|d| (d.0, d.1))
            .collect();
        (graph, placed, left, true)
    };
    // a stretch that was an OpenStreetMap bridge and is none any more, not even by a dip bridge
    for id in lost_bridges(&built, &graph) {
        placed.net.warnings.push(format!(
            "bridge on {id}: no room for a bridge model between the junctions"
        ));
    }
    Ok(RoadPlan {
        graph,
        placed,
        first_round,
        dips,
        placed_twice,
    })
}

fn place_roads(
    read: &Read,
    lakes: &[LandPolygon],
    base: &Heightmap,
    frame: &MapGeo,
    tools: &Tools,
    progress: &dyn Fn(&str),
) -> Result<Roads, BuildError> {
    let features = &read.features;
    if !features
        .iter()
        .any(|f| matches!(f.kind, FeatureKind::Road { .. }))
    {
        return Ok(Roads {
            net: PlacedNetwork::default(),
            graph: RoadGraph::default(),
            profiles: Profiles::default(),
            junctions: Junctions::default(),
            conformed: None,
            catalog: None,
            dips: Vec::new(),
        });
    }
    progress("Reading the road pieces of the game");
    let catalog = road_catalog(tools, &tools.cache_dir)?;
    progress("Placing the roads");
    let plan = plan_roads(
        features,
        &catalog,
        base,
        &read.water,
        lakes,
        frame,
        progress,
    )?;
    progress("Shaping the terrain to the roads");
    let Placed {
        net,
        junctions,
        profiles,
        ..
    } = plan.placed;
    let patches = level_patches(&junctions, f64::from(base.cell_m));
    let conformed = conform(base, &profiles.strips, &patches)?;
    Ok(Roads {
        net,
        graph: plan.graph,
        profiles,
        junctions,
        conformed: Some(conformed),
        catalog: Some(catalog),
        dips: plan.dips,
    })
}

/// The level patches of `junctions`: a circle of the junction's `patch_m` plus one cell at its height under
/// each join and each chain end on a road. A junction piece is already a level segment of its strip and gets
/// none.
#[must_use]
pub fn level_patches(junctions: &Junctions, cell_m: f64) -> Vec<LevelPatch> {
    junctions
        .points
        .iter()
        .filter(|j| matches!(j.kind, JunctionKind::Join | JunctionKind::EndOnRoad))
        .map(|j| LevelPatch {
            at: j.at,
            radius: j.patch_m + cell_m,
            height: j.height,
            rank: j.rank,
        })
        .collect()
}

/// The deck level of every bridge object: the height of the profile at its start.
fn deck_levels(roads: &Roads) -> BTreeMap<ObjectId, f64> {
    let mut levels = BTreeMap::new();
    for strip in &roads.profiles.strips {
        for (position, &road) in strip.slots.iter().enumerate() {
            let Some(placed) = roads.net.roads.get(road) else {
                continue;
            };
            if placed.kind == PlacedKind::Bridge
                && let Some(joint) = strip.joints.get(position)
            {
                levels.insert(
                    ObjectId {
                        feature: placed.slot.stretch.clone(),
                        preset: "road".to_string(),
                        slot: placed.slot.index,
                    },
                    joint.2,
                );
            }
        }
    }
    levels
}

/// Orders the objects by id; edits that outlived their objects come last, by position.
fn sorted(mut objects: Vec<PlacedObject>) -> Vec<PlacedObject> {
    objects.sort_by(|a, b| {
        a.id.is_none()
            .cmp(&b.id.is_none())
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| a.x.total_cmp(&b.x))
            .then_with(|| a.y.total_cmp(&b.y))
            .then_with(|| a.model.cmp(&b.model))
    });
    objects
}

fn object_heights(
    objects: &[PlacedObject],
    heightmap: &Heightmap,
    decks: &BTreeMap<ObjectId, f64>,
    catalog: Option<&PieceCatalog>,
    generated: &[GeneratedObject],
) -> Vec<f32> {
    // the first generated object of each id, as a linear search would find it
    let mut by_id: BTreeMap<&ObjectId, &GeneratedObject> = BTreeMap::new();
    for object in generated {
        by_id.entry(&object.id).or_insert(object);
    }
    objects
        .iter()
        .map(|object| {
            let ground = heightmap.sample(object.x, object.y).height;
            let deck = object.id.as_ref().and_then(|id| decks.get(id));
            // a replaced bridge is no longer the bridge the deck level belongs to
            let original = object.id.as_ref().and_then(|id| by_id.get(id).copied());
            let bridge: Option<&BridgeModel> = catalog.and_then(|c| {
                c.bridges().iter().find(|b| {
                    original.is_some_and(|g| g.model == b.model) && object.model == b.model
                })
            });
            match (deck, bridge) {
                (Some(level), Some(model)) => {
                    narrow(bridge_object_height(*level, model, object.scale))
                }
                _ => ground,
            }
        })
        .collect()
}

fn narrow(v: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "world file heights are 32 bit floats"
    )]
    let narrowed = v as f32;
    narrowed
}

/// Paints the paths on the ground, along their OpenStreetMap lines.
fn paint_paths(surfaces: &mut SurfaceGrid, features: &[Feature]) {
    for feature in features {
        if let FeatureKind::Path { width_m } = feature.kind {
            surfaces.stroke(&feature.points, width_m, SurfaceClass::Soil);
        }
    }
}

/// Paints the road beds on the ground: under every road piece, junction and end cap that was placed, along
/// the piece itself (the fitted pieces may lie some metres from the OpenStreetMap line, and the arms of a
/// junction leave it at right angles), with the piece's own width and 2 m more. Bridge models
/// get no bed. A stretch that is neither a bridge nor a tunnel and has nothing placed at all gets its bed
/// along its OpenStreetMap line, as wide as the catalog's first piece of its family (or the family's
/// nominal width without a catalog).
pub fn paint_roads(
    surfaces: &mut SurfaceGrid,
    graph: &RoadGraph,
    net: &PlacedNetwork,
    catalog: Option<&PieceCatalog>,
) {
    let placed: BTreeSet<&str> = net.roads.iter().map(|r| r.slot.stretch.as_str()).collect();
    for road in &net.roads {
        if road.kind == PlacedKind::Bridge {
            continue;
        }
        surfaces.stroke(
            &[(road.start.x, road.start.y), (road.end.x, road.end.y)],
            road.width + ROAD_BED_MARGIN_M,
            SurfaceClass::Soil,
        );
    }
    for stretch in &graph.stretches {
        if stretch.tunnel || stretch.bridge || placed.contains(stretch.id.as_str()) {
            continue;
        }
        let width = catalog
            .and_then(|c| c.pieces(stretch.family).first())
            .map_or(stretch.family.width_m(), |p| p.width);
        surfaces.stroke(
            &stretch.points,
            width + ROAD_BED_MARGIN_M,
            SurfaceClass::Soil,
        );
    }
}

fn polyline_distance(points: &[(f64, f64)], at: (f64, f64)) -> f64 {
    points
        .windows(2)
        .filter_map(|pair| match pair {
            [from, to] => {
                let (dx, dy) = (to.0 - from.0, to.1 - from.1);
                let length2 = dx * dx + dy * dy;
                let along = if length2 > 0.0 {
                    (((at.0 - from.0) * dx + (at.1 - from.1) * dy) / length2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                Some((at.0 - from.0 - along * dx).hypot(at.1 - from.1 - along * dy))
            }
            _ => None,
        })
        .fold(f64::INFINITY, f64::min)
}

fn statistics(
    roads: &Roads,
    pieces: &[PieceCheck],
    steep_paths: &[naves_core::checks::SteepPathSection],
) -> RoadStats {
    let mut stats = RoadStats {
        stretches: roads.graph.stretches.len(),
        pieces: roads.net.roads.len(),
        max_cut_m: roads.conformed.as_ref().map_or(0.0, |c| c.max_cut_m),
        max_fill_m: roads.conformed.as_ref().map_or(0.0, |c| c.max_fill_m),
        ..RoadStats::default()
    };
    for stretch in &roads.graph.stretches {
        let length: f64 = stretch
            .points
            .windows(2)
            .map(|p| match p {
                [a, b] => (b.0 - a.0).hypot(b.1 - a.1),
                _ => 0.0,
            })
            .sum();
        *stats
            .km_by_family
            .entry(format!("{:?}", stretch.family).to_lowercase())
            .or_insert(0.0) += length / 1000.0;
    }
    let open: std::collections::BTreeSet<&str> =
        roads.net.gaps.iter().map(|g| g.stretch.as_str()).collect();
    stats.open = open.len();
    stats.closed = stats.stretches.saturating_sub(stats.open);
    let by_id: BTreeMap<&str, &naves_core::roadnet::Stretch> = roads
        .graph
        .stretches
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect();
    let mut deviations: Vec<f64> = roads
        .net
        .roads
        .iter()
        .filter(|r| r.kind == PlacedKind::Piece)
        .filter_map(|r| {
            by_id
                .get(r.slot.stretch.as_str())
                .map(|s| polyline_distance(&s.points, (r.end.x, r.end.y)))
        })
        .collect();
    deviations.sort_by(f64::total_cmp);
    stats.median_deviation_m = deviations.get(deviations.len() / 2).copied().unwrap_or(0.0);
    for piece in pieces {
        stats.max_grade = stats.max_grade.max(piece.grade);
        stats.largest_miss_m = stats.largest_miss_m.max(piece.miss_m);
        if piece.state == DriveState::NotDrivable {
            stats.not_drivable_m +=
                (piece.end.0 - piece.start.0).hypot(piece.end.1 - piece.start.1);
        }
    }
    stats.paths_too_steep_m = steep_paths.iter().map(|s| s.length_m).sum();
    stats
}

/// The ground decision and the warnings about it.
fn decide_ground(
    project: &Project,
    sources: &LoadedSources,
    heightmap: &Heightmap,
    progress: &dyn Fn(&str),
) -> Result<(SurfaceGrid, Vec<String>), BuildError> {
    progress("Deciding the ground");
    let decided = build_surfaces(project, heightmap, &sources.as_sources())?;
    let mut warnings = Vec::new();
    if sources.land_cover.is_some() && decided.uncovered_share > UNCOVERED_WARN_SHARE {
        warnings.push(format!(
            "{:.0} % of the map is not covered by the land cover files; the slope and altitude rules decide the \
             ground there",
            decided.uncovered_share * 100.0
        ));
    }
    if let Some(osm) = sources.osm.as_ref().filter(|o| o.skipped_rings > 0) {
        warnings.push(format!(
            "{} OpenStreetMap ring(s) were skipped because the extract cuts them or lacks nodes; the land cover \
             or the rules decide there",
            osm.skipped_rings
        ));
    }
    Ok((decided.grid, warnings))
}

/// Generates the terrain, the roads and the ground of the project in `project_dir`.
///
/// `progress` is called with a short line before every step.
///
/// # Errors
/// Any [`BuildError`]; every source is read before anything is decided, so a bad file costs nothing.
pub fn generate(
    project_dir: &Path,
    project: &Project,
    tools: &Tools,
    progress: &dyn Fn(&str),
) -> Result<Generated, BuildError> {
    if project.sources.dem.is_empty() {
        return Err(BuildError::NoDem);
    }
    progress("Reading the elevation data");
    let dem_files: Vec<PathBuf> = project
        .sources
        .dem
        .iter()
        .map(|p| project_dir.join(p))
        .collect();
    let dem = Dem::open(&dem_files)?;
    progress("Sampling the heightmap");
    let base = build_heightmap(project, &dem)?;
    let sources = LoadedSources::open(project, project_dir, progress)?;
    let frame = MapGeo::new(project).map_err(HeightmapError::from)?;
    let read = read_features(project, project_dir, &frame, progress)?;
    let overrides = Overrides::load(project_dir)?;
    let lakes = sources
        .osm
        .as_ref()
        .map_or(&[][..], |o| o.polygons.as_slice());
    let roads = place_roads(&read, lakes, &base, &frame, tools, progress)?;
    let features = read.features;

    let heightmap = roads
        .conformed
        .as_ref()
        .map_or_else(|| base.clone(), |c| c.heightmap.clone());
    let generated = road_objects(&roads.net);
    let (placed, orphans) = apply_overrides(&generated, &overrides);
    let objects = sorted(placed);
    let heights = object_heights(
        &objects,
        &heightmap,
        &deck_levels(&roads),
        roads.catalog.as_ref(),
        &generated,
    );

    let (mut surfaces, mut warnings) = decide_ground(project, &sources, &heightmap, progress)?;
    paint_paths(&mut surfaces, &features);
    paint_roads(
        &mut surfaces,
        &roads.graph,
        &roads.net,
        roads.catalog.as_ref(),
    );

    progress("Checking the roads");
    let pieces = check_roads_on_terrain(
        &roads.net,
        &heightmap,
        &base,
        &side_slopes(&roads.net, &base),
    );
    let steep_paths = check_walkable(&features, &heightmap);
    let empty = Conformed {
        heightmap: heightmap.clone(),
        max_cut_m: 0.0,
        max_fill_m: 0.0,
        conflicts: Vec::new(),
        capped: Vec::new(),
    };
    let water = sources
        .osm
        .as_ref()
        .map_or(&[][..], |o| o.polygons.as_slice());
    let violations = check_roads(
        &roads.net,
        &roads.profiles,
        &roads.junctions,
        roads.conformed.as_ref().unwrap_or(&empty),
        &pieces,
        &steep_paths,
        &orphans,
        water,
        &frame,
        &roads.dips,
    );
    warnings.extend(roads.net.warnings.iter().cloned());
    let stats = statistics(&roads, &pieces, &steep_paths);
    Ok(Generated {
        base,
        heightmap,
        surfaces,
        objects,
        heights,
        features,
        pieces,
        violations,
        stats,
        warnings,
        used_land_cover: sources.land_cover.is_some(),
        used_osm: sources.osm.is_some(),
    })
}

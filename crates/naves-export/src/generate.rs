// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Generation: everything the build decides before a file is written.
//!
//! DEM, base heightmap, the OSM ways, features, the road graph, the catalog of the game's road pieces,
//! the placement, height profiles, the terrain shaped to fit them, the objects and the user's edits, the
//! ground on the shaped terrain, the paths and the road bed painted on it, the checks and the violations.
//! Nothing here touches the project folder or runs a tool other than dayzmake for the catalog, so the editor
//! can run it too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use naves_core::bridgefit::mark_placed;
use naves_core::bridgeground::bridge_spans;
use naves_core::bridgekind::{BridgeFit, choose, crossing_thickness, fit_bridges};
use naves_core::catalog::{BridgeModel, PieceCatalog};
use naves_core::checks::{DriveState, PieceCheck, check_roads_on_terrain, check_walkable};
use naves_core::conform::{AreaPatch, Conformed, LevelPatch, conform_areas};
use naves_core::dem::Dem;
use naves_core::dips::{Dip, DipAction, decide, find_dips};
use naves_core::features::{Feature, FeatureKind, road_features, water_lines};
use naves_core::heightmap::{Heightmap, HeightmapError, MapGeo, build_heightmap};
use naves_core::junctions::{Junction, JunctionKind, Junctions, add_lot_entries, find_junctions};
use naves_core::lot::{
    Assessed, LOT_INSIDE_TOL_M, LOTS_ENABLED, LotArea, LotReport, LotSurface, SurfaceSource,
    contains, edge_distance,
};
use naves_core::netclean::{self, CleanReport, CutAccess, surviving_entries};
use naves_core::objects::{
    GeneratedObject, ObjectId, Overrides, PlacedObject, apply_overrides, bridge_object_height,
    panel_object_height, road_object_id, road_objects,
};
use naves_core::osm::{LandPolygon, OsmWays, read_ways};
use naves_core::profile::{Profiles, solve_clearing};
use naves_core::project::Project;
use naves_core::roadnet::{
    DroppedDuplicate, RoadGraph, build_graph, collapse_small_rings, drop_duplicates,
    with_bridge_spans,
};
use naves_core::roadplace::{PlacedKind, PlacedNetwork, longest_bridge, place_network};
use naves_core::sideslope::side_slopes;
use naves_core::smooth::{
    SMOOTH_ENABLED, SmoothParams, SmoothReport, WaterTest, min_radius, smooth_graph_in_water,
};
use naves_core::surface::{LoadedSources, SurfaceClass, SurfaceError, SurfaceGrid, build_surfaces};
use naves_core::validate::{Violation, check_roads, lot_violations, sort_violations, water_test};
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

/// Which of the stages that change the roads run. The switches are constants, not project settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stages {
    /// Smooth the lines of the roads before the bridges are fitted (see [`SMOOTH_ENABLED`]).
    pub smooth: bool,
    /// Give the lots (car parks, camp sites and similar areas) their surface and smooth ground, and take the roads
    /// inside them out (see [`LOTS_ENABLED`]).
    pub lots: bool,
}

impl Stages {
    /// The switches of this release: both stages are off until they were judged.
    pub const DEFAULT: Stages = Stages {
        smooth: SMOOTH_ENABLED,
        lots: LOTS_ENABLED,
    };
}

impl Default for Stages {
    fn default() -> Stages {
        Stages::DEFAULT
    }
}

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
    /// What every OpenStreetMap bridge was built from, and why, in the order of their stretches.
    pub bridges: Vec<BridgeFit>,
    /// What the smoothing of the road graph did; `None` when the stage is off or there are no roads.
    pub smooth: Option<SmoothReport>,
    /// What the lot stage found and decided; `None` when the stage is off.
    pub lots: Option<LotReport>,
    /// What the clean-up of the road graph did (empty when there are no roads).
    pub clean: CleanReport,
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
/// A stretch that cannot be fitted, or [`BuildError::Profile`] when the height solve reports no solution or
/// finds a road without ground data.
pub fn place_once(
    graph: &RoadGraph,
    catalog: &PieceCatalog,
    base: &Heightmap,
) -> Result<Placed, BuildError> {
    place_once_with(graph, catalog, base, &[], &[])
}

/// [`place_once`] with the smoothed lots and the cut access roads of the lot step: an entry becomes a
/// [`JunctionKind::LotEntry`] at the placed end of its access road, held to the lot's surface, and the ground target
/// of a joint over a lot is the lot's surface there (see [`add_lot_entries`]).
///
/// # Errors
/// As [`place_once`].
pub fn place_once_with(
    graph: &RoadGraph,
    catalog: &PieceCatalog,
    base: &Heightmap,
    lots: &[LotSurface],
    entries: &[CutAccess],
) -> Result<Placed, BuildError> {
    let net = place_network(graph, catalog)?;
    let mut junctions = find_junctions(&net, graph, base);
    add_lot_entries(&mut junctions, &net, lots, entries, base);
    let side = side_slopes(&net, base);
    // the heights of all roads and junctions are one problem; a bridge over a road keeps its clearance in it
    // what the catalog could not measure it says in its own warnings
    let thickness = |c: &naves_core::roadnet::BridgeOverRoad| crossing_thickness(catalog, c);
    let (profiles, junctions) = solve_clearing(&net, base, &junctions, &side, graph, &thickness)?;
    Ok(Placed {
        net,
        junctions,
        side,
        profiles,
    })
}

/// The road graph of a project before the placement.
pub struct FittedGraph {
    /// The graph of the features without the parts of roads that OpenStreetMap draws twice (see
    /// [`drop_duplicates`]), cleaned (see [`netclean::clean`]) and smoothed when the stage is on (see
    /// [`smooth_graph_in_water`] and [`SMOOTH_ENABLED`]): its bridges are those of OpenStreetMap.
    pub built: RoadGraph,
    /// The same with every OpenStreetMap bridge built from the part that fits it, grown along the straight road
    /// as far as the part needs (see [`fit_bridges`]).
    pub graph: RoadGraph,
    /// The parts of roads that were dropped as drawn twice.
    pub duplicates: Vec<DroppedDuplicate>,
    /// What the clean-up of the graph did after that (see [`netclean::clean`]), and the roundabouts that were
    /// collapsed before it.
    pub clean: CleanReport,
    /// What the smoothing of the graph did; `None` when the stage is off ([`SMOOTH_ENABLED`]).
    pub smooth: Option<SmoothReport>,
    /// How long the smoothing took (zero when the stage is off); a measurement, not part of any output.
    pub smooth_time: std::time::Duration,
    /// What the lot stage found and decided; `None` when the stage is off ([`LOTS_ENABLED`]).
    pub lots: Option<LotReport>,
    /// How long the lot stage's own work took (zero when the stage is off): the assessment of the lots (the surface
    /// fits and the water share), their groups and the surface decision. The clean-up is not in it (it runs its own
    /// lot step as its first of five steps), and neither is the terrain shaping or the painting, which are one pass
    /// each with the roads and with the ground. A measurement, not part of any output.
    pub lots_time: std::time::Duration,
    /// What every OpenStreetMap bridge was built from, and why.
    pub bridges: Vec<BridgeFit>,
}

/// The graph of `features` as it is placed: small roundabouts collapsed, roads drawn twice dropped, the graph
/// cleaned ([`netclean::clean_with`], with the groups of the lots), the lots judged ([`naves_core::lot::assess`] and
/// [`naves_core::lot::surfaces_with_sources`]), smoothed ([`smooth_graph_in_water`], unless [`SMOOTH_ENABLED`] is
/// off) and every OpenStreetMap bridge built from the part that fits it on the original ground `base`.
#[must_use]
///
/// `in_water` tells whether a point of the map is in water: the smoothing does not widen a curve into it and a lot
/// mostly in water is excluded (see [`naves_core::validate::water_test`]). `lots` are the lot areas of the project
/// ([`naves_core::lot::areas`]); they are read only when [`LOTS_ENABLED`] is on.
pub fn fitted_graph(
    features: &[Feature],
    catalog: &PieceCatalog,
    base: &Heightmap,
    in_water: WaterTest<'_>,
    lots: &[LotArea],
) -> FittedGraph {
    fitted_graph_with(features, catalog, base, Stages::DEFAULT, in_water, lots)
}

/// [`fitted_graph`] with the stages switched on or off: with both off, the graph is the cleaned graph with the
/// bridges fitted, as it was before the stages existed, and `lots` is `None`.
#[must_use]
pub fn fitted_graph_with(
    features: &[Feature],
    catalog: &PieceCatalog,
    base: &Heightmap,
    stages: Stages,
    in_water: WaterTest<'_>,
    lots: &[LotArea],
) -> FittedGraph {
    // a roundabout smaller than `ROUNDABOUT_KEEP_M` or than the tightest arc of its surface is a plain crossing
    let (flat, roundabouts) =
        collapse_small_rings(&build_graph(features), &|f| catalog.tightest_arc(f));
    let (deduplicated, duplicates) = drop_duplicates(&flat);
    let cell_m = f64::from(base.cell_m);
    // The lots are judged on the graph as the roads come in (the duplicates dropped, before the clean-up takes
    // anything out), and the clean-up gets their groups in the same run: its step 0 removes the roads inside them.
    // The class comes from what that step removed, so it is decided after the clean-up, from the same list of lots
    // and the same road features the graph was built from.
    let started_lots = std::time::Instant::now();
    let (assessed, surfaces): (Vec<Assessed>, Vec<LotSurface>) = if stages.lots {
        naves_core::lot::assess(lots, base, in_water)
    } else {
        (Vec::new(), Vec::new())
    };
    let groups = if stages.lots {
        netclean::lot::groups(&assessed)
    } else {
        Vec::new()
    };
    // the clean-up runs between the lot work and is timed out of it: its lot step is its first, but it is one run
    // with the other four steps and cannot be separated from them
    let started_clean = std::time::Instant::now();
    let (built, mut clean) = netclean::clean_with(&deduplicated, &|f| f.width_m(), cell_m, &groups);
    let clean_time = started_clean.elapsed();
    clean.roundabouts = roundabouts;
    let decided = if stages.lots {
        naves_core::lot::surfaces_with_sources(&assessed, &deduplicated, &clean, cell_m, features)
    } else {
        Vec::new()
    };
    let lots_time = if stages.lots {
        started_lots.elapsed().saturating_sub(clean_time)
    } else {
        std::time::Duration::ZERO
    };
    let (classes, sources): (Vec<(String, SurfaceClass)>, Vec<SurfaceSource>) = decided
        .into_iter()
        .map(|(id, class, source)| ((id, class), source))
        .unzip();
    let lot_report = stages.lots.then_some(LotReport {
        assessed,
        surfaces,
        classes,
        sources,
    });
    // a family without a curve piece has nothing that counts as tight
    let started = std::time::Instant::now();
    let (built, smooth) = if stages.smooth {
        let radius = |s: &naves_core::roadnet::Stretch| {
            min_radius(catalog, s.family).unwrap_or(f64::INFINITY)
        };
        let (smoothed, report) =
            smooth_graph_in_water(&built, &radius, in_water, &SmoothParams::default());
        (smoothed, Some(report))
    } else {
        (built, None)
    };
    let smooth_time = started.elapsed();
    let ground = |p: (f64, f64)| f64::from(base.sample(p.0, p.1).height);
    let (graph, bridges) = fit_bridges(&built, &|site| choose(catalog, site), &ground);
    FittedGraph {
        built,
        graph,
        duplicates,
        clean,
        smooth,
        smooth_time,
        lots: lot_report,
        lots_time,
        bridges,
    }
}

/// The length of the bridge model a bridge over a dip on a road of `family` gets.
fn model_length(catalog: &PieceCatalog, family: naves_core::catalog::Family) -> Option<f64> {
    longest_bridge(catalog, family).map(|model| model.length)
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
/// over water. `lots` are the lot areas ([`naves_core::lot::areas`]); with the lot stage on
/// ([`Stages::DEFAULT`]), their entry junctions and through-road ground targets go into the placement (see
/// [`place_once_with`]).
///
/// # Errors
/// A stretch that cannot be fitted, or [`BuildError::Profile`] when the height solve reports no solution or
/// finds a road without ground data.
#[expect(
    clippy::too_many_arguments,
    reason = "one input per source and switch, as the placement needs them"
)]
pub fn plan_roads(
    features: &[Feature],
    catalog: &PieceCatalog,
    base: &Heightmap,
    water: &[Vec<(f64, f64)>],
    lakes: &[LandPolygon],
    frame: &MapGeo,
    lots: &[LotArea],
    progress: &dyn Fn(&str),
) -> Result<RoadPlan, BuildError> {
    plan_roads_with(
        features,
        catalog,
        base,
        water,
        lakes,
        frame,
        Stages::DEFAULT,
        lots,
        progress,
    )
}

/// [`plan_roads`] with the stages switched on or off.
///
/// # Errors
/// As [`plan_roads`].
#[expect(clippy::too_many_arguments, reason = "one input per source and switch")]
pub fn plan_roads_with(
    features: &[Feature],
    catalog: &PieceCatalog,
    base: &Heightmap,
    water: &[Vec<(f64, f64)>],
    lakes: &[LandPolygon],
    frame: &MapGeo,
    stages: Stages,
    lots: &[LotArea],
    progress: &dyn Fn(&str),
) -> Result<RoadPlan, BuildError> {
    let FittedGraph {
        graph: widened,
        duplicates,
        clean,
        smooth,
        smooth_time,
        lots: lot_report,
        lots_time,
        bridges,
        ..
    } = fitted_graph_with(
        features,
        catalog,
        base,
        stages,
        &water_test(lakes, frame),
        lots,
    );
    if smooth.is_some() {
        progress(&format!(
            "Smoothing the road lines ({:.1} s)",
            smooth_time.as_secs_f64()
        ));
    }
    if lot_report.is_some() {
        progress(&format!(
            "Shaping the lots ({:.1} s)",
            lots_time.as_secs_f64()
        ));
    }
    // an entry whose node a later clean-up step took away gets no junction (the count is in the clean-up's report).
    // The filter runs on the graph the placement gets: the smoothing and the bridge fitting come after the clean-up,
    // and an entry they dropped would be filtered here but not counted there.
    let surfaces: &[LotSurface] = lot_report
        .as_ref()
        .map_or(&[], |report| report.surfaces.as_slice());
    let entries = surviving_entries(&widened, &clean.lot_access);
    let first = place_once_with(&widened, catalog, base, surfaces, &entries)?;
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
        // a span splits a stretch and adds nodes; an entry's node stays as it is
        let placed = place_once_with(&graph, catalog, base, surfaces, &entries)?;
        // the dips that are left cannot be bridged any more
        let left = dips(&graph, &placed, false)
            .into_iter()
            .map(|d| (d.0, d.1))
            .collect();
        (graph, placed, left, true)
    };
    placed.net.duplicates = duplicates;
    placed.net.clean = clean;
    placed.net.smooth = smooth;
    placed.net.lots = lot_report;
    placed.net.bridge_fits = bridges;
    mark_placed(&mut placed.net.bridge_fits, &placed.net.roads);
    Ok(RoadPlan {
        graph,
        placed,
        first_round,
        dips,
        placed_twice,
    })
}

#[expect(clippy::too_many_arguments, reason = "one input per source and switch")]
fn place_roads(
    read: &Read,
    lots: &[LotArea],
    lakes: &[LandPolygon],
    base: &Heightmap,
    frame: &MapGeo,
    tools: &Tools,
    stages: Stages,
    progress: &dyn Fn(&str),
) -> Result<Roads, BuildError> {
    let features = &read.features;
    let has_roads = features
        .iter()
        .any(|f| matches!(f.kind, FeatureKind::Road { .. }));
    if !has_roads {
        // the stage is on: the lots (or their absence) are reported like any other build, never as "off"
        if stages.lots {
            return lots_only(lots, features, lakes, base, frame, progress);
        }
        return Ok(empty_roads());
    }
    progress("Reading the road pieces of the game");
    let catalog = road_catalog(tools, &tools.cache_dir)?;
    progress("Placing the roads");
    let plan = plan_roads_with(
        features,
        &catalog,
        base,
        &read.water,
        lakes,
        frame,
        stages,
        lots,
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
    // the lots' smooth ground is a polygon patch beside the discs of the junctions
    let lot_patches = net
        .lots
        .as_ref()
        .map_or_else(Vec::new, |report| area_patches(&report.surfaces));
    // the ground under every placed deck is shaped for it, from the deck as placed
    let spans = bridge_spans(&net, &profiles.strips, &catalog, &plan.graph);
    let conformed = conform_areas(base, &profiles.strips, &patches, &lot_patches, &spans)?;
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

/// The roads of a project that has none: nothing placed, nothing shaped.
fn empty_roads() -> Roads {
    Roads {
        net: PlacedNetwork::default(),
        graph: RoadGraph::default(),
        profiles: Profiles::default(),
        junctions: Junctions::default(),
        conformed: None,
        catalog: None,
        dips: Vec::new(),
    }
}

/// The lots of a project without any road: their surface classes and their smooth ground, without a placement. The
/// class is decided like the build's, on an empty graph, so a lot with no road of its own is grass unless its tag
/// says otherwise.
fn lots_only(
    lots: &[LotArea],
    features: &[Feature],
    lakes: &[LandPolygon],
    base: &Heightmap,
    frame: &MapGeo,
    progress: &dyn Fn(&str),
) -> Result<Roads, BuildError> {
    let started = std::time::Instant::now();
    let (assessed, surfaces) = naves_core::lot::assess(lots, base, &water_test(lakes, frame));
    let clean = CleanReport::default();
    let decided = naves_core::lot::surfaces_with_sources(
        &assessed,
        &RoadGraph::default(),
        &clean,
        f64::from(base.cell_m),
        features,
    );
    let (classes, sources): (Vec<(String, SurfaceClass)>, Vec<SurfaceSource>) = decided
        .into_iter()
        .map(|(id, class, source)| ((id, class), source))
        .unzip();
    let report = LotReport {
        assessed,
        surfaces,
        classes,
        sources,
    };
    let patches = area_patches(&report.surfaces);
    let conformed = conform_areas(base, &[], &[], &patches, &[])?;
    progress(&format!(
        "Shaping the lots ({:.1} s)",
        started.elapsed().as_secs_f64()
    ));
    Ok(Roads {
        net: PlacedNetwork {
            lots: Some(report),
            ..PlacedNetwork::default()
        },
        conformed: Some(conformed),
        ..empty_roads()
    })
}

/// The patches of `junctions`: a circle of the junction's `patch_m` plus one cell on its plane (its height and
/// gradient) under each join, overlay and chain end on a road. A junction piece is already a segment of its
/// strip and gets none.
#[must_use]
pub fn level_patches(junctions: &Junctions, cell_m: f64) -> Vec<LevelPatch> {
    // a junction on a roundabout lies on the ring's plane and gets no patch of its own
    let on_ring = |j: &Junction| {
        junctions
            .rings
            .iter()
            .any(|ring| !ring.stretches.is_disjoint(&j.stretches))
    };
    let mut patches: Vec<LevelPatch> = junctions
        .points
        .iter()
        .filter(|j| {
            matches!(
                j.kind,
                JunctionKind::Join | JunctionKind::Overlay | JunctionKind::EndOnRoad
            ) && !on_ring(j)
        })
        .map(|j| LevelPatch {
            at: j.at,
            radius: j.patch_m + cell_m,
            height: j.height,
            gradient: j.gradient,
            inner_radius: 0.0,
            rank: j.rank,
        })
        .collect();
    // every roundabout is one tilted plane over the band of its road (the ground inside it keeps its height)
    patches.extend(junctions.rings.iter().map(|ring| LevelPatch {
        at: ring.at,
        radius: ring.radius + cell_m,
        inner_radius: (ring.inner_radius - cell_m).max(0.0),
        height: ring.height,
        gradient: ring.gradient,
        rank: ring.rank,
    }));
    patches
}

/// The polygon patches of the smoothed lots: one per group, rank 0 (below every road), the ground shaped to the
/// group's surface (see [`naves_core::conform::AreaPatch`]). The order is the order of `lots`.
#[must_use]
pub fn area_patches(lots: &[LotSurface]) -> Vec<AreaPatch> {
    lots.iter()
        .cloned()
        .map(|surface| AreaPatch { surface, rank: 0 })
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
                // every object of a step of a panel grid lies at the step's level
                for column in 0..placed.abreast.max(1) {
                    levels.insert(road_object_id(placed, column), joint.2);
                }
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
            // a panel is the object the grid was laid with, at its own scale
            let panel = catalog.is_some_and(|c| {
                c.panels().iter().any(|p| {
                    original.is_some_and(|g| g.model == p.model) && object.model == p.model
                })
            });
            match (deck, bridge) {
                (Some(level), Some(model)) => {
                    narrow(bridge_object_height(*level, model, object.scale))
                }
                (Some(level), None) if panel => narrow(panel_object_height(*level, object.scale)),
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

/// Whether a paved lot is painted as [`SurfaceClass::Tarmac`]: the fifth ground layer, layer slot 4 with the mask
/// value (0, 0, 0, 128). The value is fitted to Livonia's terrain masks but has not been seen in the game yet, so
/// the design keeps one place where a paved lot is painted soil instead — this constant and [`lot_classes`] — and it
/// stays on until the game check of a map with lots disproves the slot.
pub const TARMAC_PAINTED: bool = true;

/// The classes the lots are painted with, from the decided ones: [`SurfaceClass::Tarmac`] becomes
/// [`SurfaceClass::Soil`] when `tarmac_painted` is off. The single fallback for a slot 4 that does not work (see
/// [`TARMAC_PAINTED`]); the order and the ids stay as they are.
#[must_use]
pub fn lot_classes(
    classes: &[(String, SurfaceClass)],
    tarmac_painted: bool,
) -> Vec<(String, SurfaceClass)> {
    classes
        .iter()
        .map(|(id, class)| {
            let painted = if !tarmac_painted && *class == SurfaceClass::Tarmac {
                SurfaceClass::Soil
            } else {
                *class
            };
            (id.clone(), painted)
        })
        .collect()
}

/// Paints the lots on the ground: every lot of `areas` that has a class in `decided` (the outer ring less its holes,
/// even-odd, one `SurfaceGrid::fill_polygon` each), the largest first and in the order of the ids among equals, so
/// that a car park inside a camp site shows on top of it. The order of `areas` and `decided` does not matter. A lot
/// without a decision is not painted and a decision without a lot is ignored.
pub fn paint_lots(
    surfaces: &mut SurfaceGrid,
    areas: &[LotArea],
    decided: &[(String, SurfaceClass)],
) {
    let mut lots: Vec<(&LotArea, SurfaceClass)> = areas
        .iter()
        .filter_map(|area| {
            decided
                .iter()
                .find(|(id, _)| *id == area.id)
                .map(|(_, class)| (area, *class))
        })
        .collect();
    lots.sort_by(|a, b| {
        b.0.area_m2
            .total_cmp(&a.0.area_m2)
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    for (area, class) in lots {
        surfaces.fill_polygon(&area.outer, &area.holes, class);
    }
}

/// Whether every point of a path, sampled every metre along the line (its vertices included), lies in some lot of
/// `lots` or within [`LOT_INSIDE_TOL_M`] of its edge. It may pass from one lot into another that touches it: the lots
/// of one group share one surface.
fn path_in_lot(points: &[(f64, f64)], lots: &[LotArea]) -> bool {
    let mut samples: Vec<(f64, f64)> = points.to_vec();
    for pair in points.windows(2) {
        if let [a, b] = pair {
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            if !length.is_finite() {
                return false;
            }
            let steps = length.ceil().clamp(1.0, 1.0e5);
            let mut i = 1.0;
            while i < steps {
                let f = i / steps;
                samples.push((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f));
                i += 1.0;
            }
        }
    }
    !samples.is_empty()
        && samples.iter().all(|&p| {
            lots.iter()
                .any(|lot| contains(lot, p) || edge_distance(lot, p) <= LOT_INSIDE_TOL_M)
        })
}

/// Paints the paths with lots: a path that is a street of a lot (`street: true`: a driveway or a parking aisle) and lies
/// wholly in the lots (every point of its line, not only its vertices, in some lot of `lots` or within
/// [`LOT_INSIDE_TOL_M`] of its edge) is not painted, the lot's own surface is the ground there. A street may pass from
/// one lot into another that touches it. Every other path, a footway through a lot included, a street that leaves the
/// lots and one that runs between two lots with a gap, is painted as before. `lots` are the lots that are painted (those
/// with a decision): pass no lot that is excluded. Paint the lots first.
pub fn paint_paths_with(surfaces: &mut SurfaceGrid, features: &[Feature], lots: &[LotArea]) {
    for feature in features {
        if let FeatureKind::Path { width_m, street } = feature.kind {
            if street && path_in_lot(&feature.points, lots) {
                continue;
            }
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

/// Generates the terrain, the roads and the ground of the project in `project_dir`, with the stages of this release
/// ([`Stages::DEFAULT`]).
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
    generate_with(project_dir, project, tools, Stages::DEFAULT, progress)
}

/// [`generate`] with the stages switched on or off. The lots are read from the same OpenStreetMap extracts the
/// ground is decided from ([`naves_core::lot::areas`]) and only when their stage is on.
///
/// # Errors
/// As [`generate`].
#[expect(
    clippy::too_many_lines,
    reason = "the pipeline is one list of steps, in the order they run, and each is a few lines"
)]
pub fn generate_with(
    project_dir: &Path,
    project: &Project,
    tools: &Tools,
    stages: Stages,
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
    // the lot areas of the same extracts, map metres and stable ids
    let (lots, skipped_lots) = if stages.lots {
        sources
            .osm
            .as_ref()
            .map_or((Vec::new(), 0), |land| naves_core::lot::areas(land, &frame))
    } else {
        (Vec::new(), 0)
    };
    // rings the reader of the extract could not close or that have no three corners, beside those `lot::areas`
    // skips for a point that cannot be projected
    let skipped_lot_rings = if stages.lots {
        sources
            .osm
            .as_ref()
            .map_or(0, |land| land.skipped_lot_rings)
    } else {
        0
    };
    let roads = place_roads(&read, &lots, lakes, &base, &frame, tools, stages, progress)?;
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
    let skipped = skipped_lots + skipped_lot_rings;
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} OpenStreetMap lot ring(s) were skipped: the extract cuts them, they lack nodes, have fewer \
             than three corners or a point that cannot be projected; they get no surface"
        ));
    }
    // the decided ground, then the lots (largest first), then the paths (a street that lies in painted lots is not
    // painted), then the road beds. The classes are the ones `paint_lots` is given: exactly the lots with a decision.
    let classes = roads.net.lots.as_ref().map_or_else(Vec::new, |report| {
        lot_classes(&report.classes, TARMAC_PAINTED)
    });
    paint_lots(&mut surfaces, &lots, &classes);
    let painted: Vec<LotArea> = lots
        .iter()
        .filter(|area| classes.iter().any(|(id, _)| *id == area.id))
        .cloned()
        .collect();
    paint_paths_with(&mut surfaces, &features, &painted);
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
        buried: Vec::new(),
        floating: Vec::new(),
    };
    let water = sources
        .osm
        .as_ref()
        .map_or(&[][..], |o| o.polygons.as_slice());
    let mut violations = check_roads(
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
    if let Some(report) = roads.net.lots.as_ref() {
        violations.extend(lot_violations(report, &roads.net.clean));
        sort_violations(&mut violations);
    }
    if let Some(catalog) = roads.catalog.as_ref() {
        warnings.extend(catalog.warnings().iter().cloned());
    }
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
        bridges: roads.net.bridge_fits.clone(),
        smooth: roads.net.smooth.clone(),
        lots: roads.net.lots.clone(),
        clean: roads.net.clean.clone(),
    })
}

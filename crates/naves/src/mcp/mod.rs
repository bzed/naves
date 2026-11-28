// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The MCP server (`naves mcp <dir>`): the project, the heightmap, top-down renders and the build for AI
//! clients, over standard input and output (local only).
//!
//! Requests are untrusted input: paths and values go through [`naves_core::project::Project::with_value`],
//! the render request is validated by `naves-core`, nothing panics, and failures are tool errors with a
//! message, never a dead server. Blocking work (DEM reading, rendering, building) runs on the blocking
//! thread pool.

use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use naves_core::heightmap::{Heightmap, MapGeo};
use naves_core::imagery::TileImagery;
use naves_core::imagery::catalog::{SourceKind, catalog, options_for, user_catalog_path};
use naves_core::render::{
    LayerData, MapLayer, Overlay, RenderRequest, composite, render_layer, render_topdown,
};

use naves_core::edits::{
    By, EditsError, ElementId, MAX_LISTED_WAYS, NodeIndex, WayAction, WayEdits,
};
use naves_core::osm::OsmWay;
use naves_core::validate::Rule;
use naves_export::build::BuildOutput;
use naves_export::generate::Source;
use naves_export::imagery_fetch::FetchRequest;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;

pub mod host;
pub mod roads;
use host::{Host, MapData, MapNeeds, StandaloneHost, ToolLocator, blocking};

/// Widest picture `render_topdown` returns.
const MAX_RENDER_WIDTH: u32 = 2048;

/// Arguments of `project_set`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetParams {
    /// Dotted path of the field, for example `scale.height` or `sources.dem`.
    path: String,
    /// The new value as JSON.
    value: serde_json::Value,
}

/// Overlay of `render_topdown`.
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OverlayArg {
    /// Shaded relief.
    Hillshade,
    /// Slope colours.
    Slope,
    /// Grey by height.
    Height,
    /// The ground class of every cell.
    Surface,
    /// The satellite or aerial imagery of sources.imagery; ground without imagery is checkered.
    Imagery,
}

impl From<OverlayArg> for Overlay {
    fn from(arg: OverlayArg) -> Self {
        match arg {
            OverlayArg::Hillshade => Overlay::Hillshade,
            OverlayArg::Slope => Overlay::Slope,
            OverlayArg::Height => Overlay::Height,
            OverlayArg::Surface => Overlay::Surface,
            OverlayArg::Imagery => Overlay::Imagery,
        }
    }
}

/// Arguments of `render_topdown`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RenderParams {
    /// West edge in map metres.
    x0: f64,
    /// South edge in map metres.
    y0: f64,
    /// East edge in map metres.
    x1: f64,
    /// North edge in map metres.
    y1: f64,
    /// Output width in pixels (at most 2048); the height follows from the rectangle.
    px: u32,
    /// What to draw.
    overlay: OverlayArg,
    /// Layers to draw over it, by id: `terrain_changes`, `too_steep_to_walk`, `roads`, `paths`, `drivable`. The road
    /// layers generate the roads, which needs dayzmake and the DayZ Server install. Drawn in a fixed order.
    layers: Option<Vec<String>>,
}

/// Arguments of `validation_list`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ValidationParams {
    /// Only the violations of this rule, for example `road_not_drivable`.
    rule: Option<String>,
}

/// Arguments of `imagery_fetch`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FetchParams {
    /// The id of a source from `imagery_options`. Only catalogue ids are accepted, never an address.
    id: String,
    /// First day of the search for scenes, `YYYY-MM-DD` (needs `to`).
    from: Option<String>,
    /// Last day of the search for scenes, `YYYY-MM-DD` (needs `from`).
    to: Option<String>,
}

/// The server state.
#[derive(Clone)]
pub struct NavesMcp {
    host: Arc<Host>,
    tool_router: ToolRouter<Self>,
}

fn success(text: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text)])
}

fn failure(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text)])
}

/// The way `id` as it is now (second) and as it was before any edit (first): the extract's way for an OSM id,
/// the way as drawn for a user id. Refuses an unknown way, one that is not a road (the source also holds
/// streams and rivers) and one whose edit deletes it with `deleted`.
fn find_way<'a>(
    edits: &WayEdits,
    source: &'a Source,
    id: ElementId,
    deleted: &'static str,
) -> Result<(&'a OsmWay, &'a OsmWay), EditsError> {
    if edits
        .ways
        .iter()
        .any(|w| w.id == id && w.action == WayAction::Delete)
    {
        return Err(roads::refused(id, deleted));
    }
    let current = id
        .raw()
        .and_then(|raw| roads::way_by_raw(&source.ways, raw))
        .ok_or_else(|| roads::refused(id, "is unknown"))?;
    if !current.tags.contains_key("highway") {
        return Err(roads::refused(id, "is not a road"));
    }
    let original = if id.is_user() {
        current
    } else {
        roads::way_by_raw(&source.extract, current.id)
            .ok_or_else(|| roads::refused(id, "is unknown"))?
    };
    Ok((original, current))
}

/// The answer of an edit that returns a way: the way as it is after it.
fn road_result(result: Result<(ElementId, Arc<Source>), String>) -> CallToolResult {
    match result.and_then(|(id, source)| roads::way_json_of(&source, id)) {
        Ok(value) => json_text(&value),
        Err(e) => failure(e),
    }
}

fn json_text(value: &serde_json::Value) -> CallToolResult {
    success(value.to_string())
}

/// The layers named in a request, in the order they are drawn.
fn parse_layers(ids: Option<&[String]>) -> Result<Vec<MapLayer>, String> {
    let Some(ids) = ids else {
        return Ok(Vec::new());
    };
    for id in ids {
        if !MapLayer::ALL.iter().any(|l| l.id() == id) {
            let known: Vec<&str> = MapLayer::ALL.iter().map(|l| l.id()).collect();
            return Err(format!(
                "unknown layer {id:?}; the layers are: {}",
                known.join(", ")
            ));
        }
    }
    Ok(MapLayer::ALL
        .iter()
        .filter(|l| ids.iter().any(|id| id == l.id()))
        .copied()
        .collect())
}

/// Whether a layer is drawn from the generated roads.
fn needs_roads(layer: MapLayer) -> bool {
    matches!(
        layer,
        MapLayer::Roads | MapLayer::Paths | MapLayer::Drivable | MapLayer::TerrainChanges
    )
}

#[tool_router]
impl NavesMcp {
    /// A server for the data of `host`.
    pub fn new(host: Host) -> Self {
        Self {
            host: Arc::new(host),
            tool_router: Self::tool_router(),
        }
    }

    /// The heightmap of the saved project, computed again only when the project file has changed.
    async fn heightmap(&self) -> Result<Arc<Heightmap>, String> {
        Ok(self.host.map(MapNeeds::default()).await?.heightmap)
    }

    #[tool(
        description = "The project (naves.toml) as JSON: name, area, scales, grid sizes and sources."
    )]
    async fn project_get(&self) -> Result<CallToolResult, McpError> {
        Ok(match self.host.project().await {
            Ok(project) => match serde_json::to_value(&project) {
                Ok(value) => json_text(&value),
                Err(e) => failure(e.to_string()),
            },
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "Set one field of the project, for example path \"scale.height\" and value 0.75. The value is validated like a project file; an invalid value is an error and naves.toml stays as it was. Returns the new project."
    )]
    async fn project_set(
        &self,
        Parameters(params): Parameters<SetParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self.host.set_project_value(params.path, params.value).await;
        Ok(match result {
            Ok(project) => match serde_json::to_value(&project) {
                Ok(value) => json_text(&value),
                Err(e) => failure(e.to_string()),
            },
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "Statistics of the heightmap: lowest and highest height in metres, vertices per side, vertex distance and map size in metres. Needs DEM files in sources.dem."
    )]
    async fn heightmap_stats(&self) -> Result<CallToolResult, McpError> {
        Ok(match self.heightmap().await {
            Ok(hm) => {
                let (min, max) = hm.min_max();
                json_text(&json!({
                    "min": min,
                    "max": max,
                    "size": hm.size,
                    "cell_m": hm.cell_m,
                    "map_size_m": f64::from(hm.size) * f64::from(hm.cell_m),
                }))
            }
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "A top-down PNG of a rectangle of the map (north up). x0,y0 is the south-west and x1,y1 the north-east corner in map metres from the south-west corner of the map; px is the width in pixels (at most 2048); overlay is hillshade, slope, height, surface (the ground classes grass, rock, forest and soil in their satellite colours) or imagery (the merged images of sources.imagery, ground without imagery checkered)."
    )]
    async fn render_topdown(
        &self,
        Parameters(params): Parameters<RenderParams>,
    ) -> Result<CallToolResult, McpError> {
        let layers = match parse_layers(params.layers.as_deref()) {
            Ok(layers) => layers,
            Err(e) => return Ok(failure(e)),
        };
        let wants_surfaces = matches!(params.overlay, OverlayArg::Surface);
        let wants_imagery = matches!(params.overlay, OverlayArg::Imagery);
        let wants_roads = layers.iter().any(|l| needs_roads(*l));
        let MapData {
            project,
            heightmap: hm,
            surfaces,
            imagery,
            roads,
        } = match self
            .host
            .map(MapNeeds {
                surfaces: wants_surfaces,
                imagery: wants_imagery,
                roads: wants_roads,
            })
            .await
        {
            Ok(map) => map,
            Err(e) => return Ok(failure(e)),
        };
        let request = RenderRequest {
            x0: params.x0,
            y0: params.y0,
            x1: params.x1,
            y1: params.y1,
            px: params.px.min(MAX_RENDER_WIDTH),
            overlay: params.overlay.into(),
        };
        let png = blocking(move || -> Result<Vec<u8>, String> {
            let tile_imagery = imagery.as_deref().map(|loaded| TileImagery {
                composite: &loaded.composite,
                project: &project,
            });
            let mut image =
                render_topdown(&hm, &request, surfaces.as_deref(), tile_imagery.as_ref())
                    .map_err(|e| e.to_string())?;
            let data = match roads.as_deref() {
                Some(roads) => LayerData {
                    heightmap: &roads.heightmap,
                    base: Some(&roads.base),
                    features: &roads.features,
                    pieces: &roads.pieces,
                },
                None => LayerData::new(&hm),
            };
            for layer in &layers {
                let over = render_layer(*layer, &data, &request).map_err(|e| e.to_string())?;
                image = composite(&image, &over);
            }
            let mut bytes = Cursor::new(Vec::new());
            image
                .write_to(&mut bytes, image::ImageFormat::Png)
                .map_err(|e| e.to_string())?;
            Ok(bytes.into_inner())
        })
        .await;
        Ok(match png.and_then(|r| r) {
            Ok(bytes) => CallToolResult::success(vec![ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(bytes),
                "image/png",
            )]),
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "The roads and paths (OpenStreetMap ways with a highway tag, edits applied) that touch a rectangle of the map in map metres: id, origin (osm, edited, new), tags and points. At most 1000 ways (limit), `more` is true when there are more. A way that crosses the edge of the map can have points outside it (0..map size); cut such points at the edge before road_modify, which refuses points outside the map."
    )]
    async fn roads_list(
        &self,
        Parameters(params): Parameters<roads::RectParams>,
    ) -> Result<CallToolResult, McpError> {
        let rect = (params.x0, params.y0, params.x1, params.y1);
        let finite = [params.x0, params.y0, params.x1, params.y1]
            .iter()
            .all(|v| v.is_finite());
        if !finite || params.x1 <= params.x0 || params.y1 <= params.y0 {
            return Ok(failure(
                "the bounds must be finite with x0 < x1 and y0 < y1",
            ));
        }
        if params.limit == Some(0) {
            return Ok(failure("the limit must be at least 1"));
        }
        let limit = params
            .limit
            .and_then(|l| usize::try_from(l).ok())
            .map_or(MAX_LISTED_WAYS, |l| l.min(MAX_LISTED_WAYS));
        let source = match self.host.source().await {
            Ok(source) => source,
            Err(e) => return Ok(failure(e)),
        };
        let listed = blocking(move || {
            let (edited, moved) = roads::edit_sets(&source);
            let mut ways = Vec::new();
            let mut more = false;
            for way in &source.ways.ways {
                if !way.tags.contains_key("highway") {
                    continue;
                }
                let points: Vec<(f64, f64)> = way
                    .points
                    .iter()
                    .filter_map(|&(lat, lon)| source.frame.to_map(lat, lon).ok())
                    .filter(|(x, y)| x.is_finite() && y.is_finite())
                    .collect();
                if !roads::touches(&points, rect) {
                    continue;
                }
                if ways.len() == limit {
                    more = true;
                    break;
                }
                ways.push(roads::way_json(&source, way, &edited, &moved));
            }
            json!({"ways": ways, "more": more})
        })
        .await;
        Ok(match listed {
            Ok(value) => json_text(&value),
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "Adds a road or path: `points` are [x, y] in map metres (at least two, at most 2000, inside the map), `tags` the OpenStreetMap tags (a highway tag is required). A point within 5 cm of an existing node is that node, so the road joins what it touches; a junction point that is moved leaves the other roads where they were, so keep junction points unchanged to stay joined. The edit is written to edits/ways.toml at once; there is no undo here. Returns the new way as roads_list shows it. A build is needed before the roads in the game change."
    )]
    async fn road_add(
        &self,
        Parameters(params): Parameters<roads::AddParams>,
    ) -> Result<CallToolResult, McpError> {
        let points = roads::tuples(&params.points);
        let tags = params.tags;
        let result = self
            .host
            .edit_ways(move |edits, source| {
                let index = NodeIndex::new(&source.ways, &source.frame);
                edits.add_line(&index, &source.frame, tags, &points, By::Ai)
            })
            .await;
        Ok(road_result(result))
    }

    #[tool(
        description = "Changes the tags and/or the line of a road, by the id roads_list gives. What is left out stays as it is; `tags` replace all tags (a highway tag is required), `points` are [x, y] in map metres. A point within 5 cm of an existing node is that node; a junction point that is moved leaves the other roads where they were, so keep junction points unchanged to stay joined. Written to edits/ways.toml at once; no undo here. Returns the way as roads_list shows it."
    )]
    async fn road_modify(
        &self,
        Parameters(params): Parameters<roads::ModifyParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = match roads::parse_id(&params.id) {
            Ok(id) => id,
            Err(e) => return Ok(failure(e)),
        };
        let points = params.points.as_deref().map(roads::tuples);
        let tags = params.tags;
        let result = self
            .host
            .edit_ways(move |edits, source| {
                let (original, current) =
                    find_way(edits, source, id, "is deleted; road_reset brings it back")?;
                let index = NodeIndex::new(&source.ways, &source.frame);
                edits.modify_line(
                    &index,
                    &source.frame,
                    original,
                    current,
                    tags,
                    points.as_deref(),
                    By::Ai,
                )?;
                Ok(id)
            })
            .await;
        Ok(road_result(result))
    }

    #[tool(
        description = "Deletes a road, by the id roads_list gives. A way of OpenStreetMap stays deleted until road_reset; a way that was added is gone for good. Written to edits/ways.toml at once; no undo here. Returns {\"deleted\": id}."
    )]
    async fn road_delete(
        &self,
        Parameters(params): Parameters<roads::IdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = match roads::parse_id(&params.id) {
            Ok(id) => id,
            Err(e) => return Ok(failure(e)),
        };
        let result = self
            .host
            .edit_ways(move |edits, source| {
                let (original, _) = find_way(edits, source, id, "is already deleted")?;
                edits.delete_way(original, By::Ai)?;
                Ok(id)
            })
            .await;
        Ok(match result {
            Ok((id, _)) => json_text(&json!({"deleted": id.to_string()})),
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "Takes back the edit of a road, by the id roads_list gives: a way of OpenStreetMap is as OpenStreetMap has it again (moved nodes stay moved, they may belong to other roads), a way that was added is removed. Written to edits/ways.toml at once. Returns the way, or {\"removed\": id} for an added way."
    )]
    async fn road_reset(
        &self,
        Parameters(params): Parameters<roads::IdParams>,
    ) -> Result<CallToolResult, McpError> {
        let id = match roads::parse_id(&params.id) {
            Ok(id) => id,
            Err(e) => return Ok(failure(e)),
        };
        let result =
            self.host
                .edit_ways(move |edits, source| {
                    let raw = id.raw();
                    let now = raw.and_then(|raw| roads::way_by_raw(&source.ways, raw));
                    let in_extract = raw.and_then(|raw| roads::way_by_raw(&source.extract, raw));
                    if now
                        .or(in_extract)
                        .is_some_and(|w| !w.tags.contains_key("highway"))
                    {
                        return Err(roads::refused(id, "is not a road"));
                    }
                    if !edits.ways.iter().any(|w| w.id == id) {
                        let problem = match (now, in_extract) {
                            (Some(way), _)
                                if way.nodes.iter().any(|&n| {
                                    edits.nodes.iter().any(|e| e.id.raw() == Some(n))
                                }) =>
                            {
                                "has no edit of its own; moved nodes stay moved"
                            }
                            (_, Some(_)) => "has no edit",
                            _ => "is unknown",
                        };
                        return Err(roads::refused(id, problem));
                    }
                    edits.reset_way(id);
                    Ok(id)
                })
                .await;
        Ok(match result {
            // a drawn way, or an edit of an OpenStreetMap way that is not in the extract (any more)
            Ok((id, source))
                if id.is_user()
                    || id
                        .raw()
                        .and_then(|raw| roads::way_by_raw(&source.ways, raw))
                        .is_none() =>
            {
                json_text(&json!({"removed": id.to_string()}))
            }
            other => road_result(other),
        })
    }

    #[tool(
        description = "The numbers of the generated road network: kilometres per surface, pieces, stretches and how many are not closed, the median distance of the pieces from the OpenStreetMap line, the steepest grade, the deepest cut and highest fill, the length that vehicles cannot drive, the largest distance of a piece from the ground and the length of paths that are too steep to walk. Generates the roads (dayzmake and the DayZ Server install are needed). A project without OpenStreetMap files has empty numbers."
    )]
    async fn roads_stats(&self) -> Result<CallToolResult, McpError> {
        let map = match self
            .host
            .map(MapNeeds {
                roads: true,
                ..MapNeeds::default()
            })
            .await
        {
            Ok(map) => map,
            Err(e) => return Ok(failure(e)),
        };
        let stats = map
            .roads
            .as_deref()
            .map(|r| r.stats.clone())
            .unwrap_or_default();
        Ok(match serde_json::to_value(&stats) {
            Ok(value) => json_text(&value),
            Err(e) => failure(e.to_string()),
        })
    }

    #[tool(
        description = "What is wrong with the generated roads: a list of violations, each with its rule, a position in map metres and a message. Optional rule (road_not_drivable, road_misses_terrain, path_too_steep, road_grade, road_gap, road_kink, road_in_water, road_conflict, earthworks_capped, orphaned_edit, join_without_junction, tunnel_omitted, junction_too_steep, road_far_from_ground, road_over_water, road_duplicate_dropped, bridge_clearance, bridge_blocks_road, bridge_not_built, bridge_buried, bridge_post_floats, bridge_end_step, road_roundabout_collapsed, road_sidepath_dropped, road_stub_dropped, road_junctions_merged, road_junctions_not_merged, bridge_approach_made, bridge_end_at_junction, road_curve_widened, road_curve_too_tight, road_curve_planned, road_curve_not_planned, lot_smoothed, lot_surface_only, lot_excluded, lot_steep, lot_move_limited, road_in_lot_dropped, edit_broken, edit_original_changed, edit_outside_map) keeps only that rule. Generates the roads."
    )]
    async fn validation_list(
        &self,
        Parameters(params): Parameters<ValidationParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(rule) = &params.rule
            && !Rule::ALL.iter().any(|r| r.id() == rule)
        {
            let known: Vec<&str> = Rule::ALL.iter().map(|r| r.id()).collect();
            return Ok(failure(format!(
                "unknown rule {rule:?}; the rules are: {}",
                known.join(", ")
            )));
        }
        let map = match self
            .host
            .map(MapNeeds {
                roads: true,
                ..MapNeeds::default()
            })
            .await
        {
            Ok(map) => map,
            Err(e) => return Ok(failure(e)),
        };
        let violations: Vec<serde_json::Value> = map
            .roads
            .as_deref()
            .map(|r| r.violations.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|v| {
                params
                    .rule
                    .as_deref()
                    .is_none_or(|rule| v.rule.id() == rule)
            })
            .map(|v| json!({ "rule": v.rule.id(), "x": v.at.0, "y": v.at.1, "message": v.message }))
            .collect();
        Ok(json_text(&json!({ "violations": violations })))
    }

    #[tool(
        description = "Which imagery sources exist for the area of the project, best first: id, name, how much of the map it covers (0..1), pixel size in metres, licence, credit, whether it is non-commercial and a note. Whole-map sources that allow commercial use come first."
    )]
    async fn imagery_options(&self) -> Result<CallToolResult, McpError> {
        let project = match self.host.project().await {
            Ok(project) => project,
            Err(e) => return Ok(failure(e)),
        };
        let result = blocking(move || -> Result<serde_json::Value, String> {
            let window = MapGeo::new(&project)
                .and_then(|geo| geo.window(0.0))
                .map_err(|e| e.to_string())?;
            let sources = catalog(user_catalog_path().as_deref()).map_err(|e| e.to_string())?;
            let options: Vec<serde_json::Value> = options_for(&sources, window)
                .into_iter()
                .map(|o| {
                    json!({
                        "id": o.source.id,
                        "name": o.source.name,
                        "kind": match o.source.kind {
                            SourceKind::Stac { .. } => "stac",
                            SourceKind::Wms { .. } => "wms",
                        },
                        "covers": o.coverage_share,
                        "pixel_m": o.source.native_m,
                        "licence": o.source.licence,
                        "attribution": o.source.attribution,
                        "non_commercial": o.source.non_commercial,
                        "note": o.source.note,
                    })
                })
                .collect();
            Ok(json!({
                "area": {
                    "west": window.west,
                    "south": window.south,
                    "east": window.east,
                    "north": window.north,
                },
                "options": options,
            }))
        })
        .await;
        Ok(match result.and_then(|r| r) {
            Ok(value) => json_text(&value),
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "Fetch imagery from a catalogue source (an id from imagery_options) for the area of the project: the files go into the imagery folder and are listed in naves.toml with their licence. Takes only catalogue ids, no addresses. Can take minutes and download hundreds of megabytes. Returns the files and warnings."
    )]
    async fn imagery_fetch(
        &self,
        Parameters(params): Parameters<FetchParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .host
            .fetch_imagery(
                params.id,
                FetchRequest {
                    from: params.from,
                    to: params.to,
                },
            )
            .await;
        Ok(match result {
            Ok((files, warnings)) => json_text(&json!({
                "files": files.iter().map(|f| json!({
                    "path": f.path,
                    "attribution": f.attribution,
                    "licence": f.licence,
                    "non_commercial": f.non_commercial,
                })).collect::<Vec<_>>(),
                "warnings": warnings,
            })),
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "Build the mod and the server mission of the project (takes a while). Returns the mod and mission folders, warnings and the progress lines. Needs dayzmake, DayZ Tools and Proton, see the manual."
    )]
    async fn build(&self) -> Result<CallToolResult, McpError> {
        Ok(match self.host.build().await {
            Ok((out, progress)) => json_text(&build_json(&out, &progress)),
            Err(e) => failure(e),
        })
    }
}

/// What the `build` tool returns: the folders, the warnings and the progress lines, and the one-line summaries of
/// the stages. `lots` is left out when the lot stage did not run (its switch is `LOTS_ENABLED`).
fn build_json(out: &BuildOutput, progress: &[String]) -> serde_json::Value {
    let mut value = json!({
        "mod_dir": out.mod_dir,
        "mission_dir": out.mission_dir,
        "warnings": out.warnings,
        "bridges": out.bridges,
        "planning": out.planning,
        "smoothing": out.smoothing,
        "progress": progress,
    });
    if let (Some(lots), Some(object)) = (&out.lots, value.as_object_mut()) {
        object.insert("lots".to_string(), serde_json::Value::from(lots.clone()));
    }
    if let (Some(edits), Some(object)) = (&out.edits, value.as_object_mut()) {
        object.insert("edits".to_string(), serde_json::Value::from(edits.clone()));
    }
    value
}

#[expect(
    clippy::unused_async_trait_impl,
    reason = "the impl is generated by rmcp's tool_handler macro, not by us"
)]
#[tool_handler(router = self.tool_router)]
impl ServerHandler for NavesMcp {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        rmcp::model::ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(self.host.instructions().to_string())
    }
}

/// Serves the project in `dir` over standard input and output until the client disconnects.
///
/// # Errors
///
/// The transport fails or the server stops with an error.
pub async fn serve(dir: &Path) -> anyhow::Result<()> {
    let locate: ToolLocator =
        Arc::new(|| crate::user_prefs::locate_tools().map_err(|e| e.to_string()));
    let service = NavesMcp::new(Host::Standalone(StandaloneHost::new(dir, locate)))
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| anyhow::anyhow!("cannot start the MCP server: {e}"))?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn output(lots: Option<&str>) -> BuildOutput {
        BuildOutput {
            mod_dir: PathBuf::from("/mod"),
            mission_dir: PathBuf::from("/mission"),
            warnings: vec!["a warning".to_string()],
            bridges: vec!["a bridge".to_string()],
            planning: None,
            smoothing: None,
            lots: lots.map(str::to_string),
            edits: None,
        }
    }

    #[test]
    fn the_build_result_has_no_lot_key_when_the_stage_did_not_run() {
        let value = build_json(&output(None), &["a progress line".to_string()]);
        assert!(value.get("lots").is_none(), "{value}");
        // exactly the keys a build had before the lot stage existed
        let keys: Vec<&String> = value
            .as_object()
            .expect("an object")
            .keys()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "bridges",
                "mission_dir",
                "mod_dir",
                "planning",
                "progress",
                "smoothing",
                "warnings"
            ]
        );
    }

    #[test]
    fn the_build_result_has_the_edits_line_when_there_was_something_to_say() {
        let mut out = output(None);
        out.edits =
            Some("Edits: 1 applied, 0 broken, 0 changed since the edit, 0 outside the map".into());
        let value = build_json(&out, &[]);
        assert!(
            value["edits"]
                .as_str()
                .unwrap()
                .starts_with("Edits: 1 applied")
        );
    }

    #[test]
    fn the_build_result_has_the_lot_line_when_the_stage_ran() {
        let value = build_json(&output(Some("Lots: none")), &[]);
        assert_eq!(value["lots"], "Lots: none");
    }
}

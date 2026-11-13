// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The MCP server (`naves mcp <dir>`): the project, the heightmap, top-down renders and the build for AI
//! clients, over standard input and output (local only).
//!
//! Requests are untrusted input: paths and values go through [`Project::with_value`], the render request is
//! validated by `naves-core`, nothing panics, and failures are tool errors with a message, never a dead
//! server. Blocking work (DEM reading, rendering, building) runs on the blocking thread pool.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use naves_core::dem::Dem;
use naves_core::heightmap::{Heightmap, MapGeo, build_heightmap};
use naves_core::imagery::catalog::{
    SourceKind, catalog, find_source, options_for, user_catalog_path,
};
use naves_core::imagery::{LoadedImagery, TileImagery, load_imagery};
use naves_core::project::Project;
use naves_core::render::{
    LayerData, MapLayer, Overlay, RenderRequest, composite, render_layer, render_topdown,
};
use naves_core::surface::{SurfaceGrid, project_surfaces};
use naves_core::validate::Rule;
use naves_export::build::build;
use naves_export::generate::{Generated, generate};
use naves_export::imagery_fetch::{FetchRequest, RealHttp, fetch};
use naves_export::tools::Tools;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;

/// Widest picture `render_topdown` returns.
const MAX_RENDER_WIDTH: u32 = 2048;

type ToolLocator = Arc<dyn Fn() -> Result<Tools, String> + Send + Sync>;

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
    /// layers generate the roads, which needs armake2 and the DayZ Server install. Drawn in a fixed order.
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

/// A heightmap, the ground surfaces and the imagery once they were asked for, and the project they were made
/// from.
struct Cached {
    project: Project,
    heightmap: Arc<Heightmap>,
    surfaces: Option<Arc<SurfaceGrid>>,
    /// Whether the imagery was looked for; `imagery` stays empty when the project lists none.
    imagery_loaded: bool,
    imagery: Option<Arc<LoadedImagery>>,
    /// Whether the roads were generated; `roads` stays empty when the project lists no OpenStreetMap file.
    roads_loaded: bool,
    roads: Option<Arc<Generated>>,
}

/// What `NavesMcp::map` hands out.
struct MapData {
    project: Project,
    heightmap: Arc<Heightmap>,
    surfaces: Option<Arc<SurfaceGrid>>,
    imagery: Option<Arc<LoadedImagery>>,
    roads: Option<Arc<Generated>>,
}

type CachedHeightmap = Option<Cached>;

/// The server state.
#[derive(Clone)]
pub struct NavesMcp {
    dir: PathBuf,
    locate_tools: ToolLocator,
    heightmap: Arc<tokio::sync::Mutex<CachedHeightmap>>,
    /// Held by what changes the project or its build folder, so that these run one at a time.
    writes: Arc<tokio::sync::Mutex<()>>,
    tool_router: ToolRouter<Self>,
}

fn success(text: String) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text)])
}

fn failure(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text)])
}

fn json_text(value: &serde_json::Value) -> CallToolResult {
    success(value.to_string())
}

/// Runs blocking work off the async threads; a panic or a cancelled task becomes an error message.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("internal error: {e}"))
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

fn compute_heightmap(dir: &Path, project: &Project) -> Result<Heightmap, String> {
    if project.sources.dem.is_empty() {
        return Err("the project has no elevation data: add DEM files to sources.dem".to_string());
    }
    let files: Vec<PathBuf> = project.sources.dem.iter().map(|p| dir.join(p)).collect();
    let dem = Dem::open(&files).map_err(|e| e.to_string())?;
    build_heightmap(project, &dem).map_err(|e| e.to_string())
}

#[tool_router]
impl NavesMcp {
    /// A server for the project in `dir`; `locate_tools` runs when a build starts.
    pub fn new(dir: &Path, locate_tools: ToolLocator) -> Self {
        Self {
            dir: dir.to_path_buf(),
            locate_tools,
            heightmap: Arc::new(tokio::sync::Mutex::new(None)),
            writes: Arc::new(tokio::sync::Mutex::new(())),
            tool_router: Self::tool_router(),
        }
    }

    /// The heightmap of the saved project, computed again only when the project file has changed.
    async fn heightmap(&self) -> Result<Arc<Heightmap>, String> {
        Ok(self.map(false, false, false).await?.heightmap)
    }

    /// The heightmap and, when asked for, the ground surfaces and the imagery of the saved project. All are
    /// computed again only when the project file has changed; surfaces and imagery are only made when asked
    /// for.
    async fn map(
        &self,
        with_surfaces: bool,
        with_imagery: bool,
        with_roads: bool,
    ) -> Result<MapData, String> {
        let mut cache = self.heightmap.lock().await;
        let dir = self.dir.clone();
        let project = blocking(move || Project::load(&dir).map_err(|e| e.to_string())).await??;
        if cache.as_ref().is_none_or(|c| c.project != project) {
            let dir = self.dir.clone();
            let input = project.clone();
            let hm = Arc::new(blocking(move || compute_heightmap(&dir, &input)).await??);
            *cache = Some(Cached {
                project: project.clone(),
                heightmap: hm,
                surfaces: None,
                imagery_loaded: false,
                imagery: None,
                roads_loaded: false,
                roads: None,
            });
        }
        let Some(cached) = cache.as_mut() else {
            return Err("internal error: no cached map".to_string());
        };
        if with_surfaces && cached.surfaces.is_none() {
            let dir = self.dir.clone();
            let hm = Arc::clone(&cached.heightmap);
            let input = project.clone();
            let grid =
                blocking(move || project_surfaces(&input, &dir, &hm).map_err(|e| e.to_string()))
                    .await??;
            cached.surfaces = Some(Arc::new(grid));
        }
        if with_imagery && !cached.imagery_loaded {
            let dir = self.dir.clone();
            let input = project.clone();
            let loaded =
                blocking(move || load_imagery(&input, &dir, &|_| {}).map_err(|e| e.to_string()))
                    .await??;
            cached.imagery = loaded.map(Arc::new);
            cached.imagery_loaded = true;
        }
        if with_roads && !cached.roads_loaded {
            if !project.sources.osm.is_empty() {
                let tools =
                    (self.locate_tools)().map_err(|e| format!("Roads need armake2: {e}"))?;
                let dir = self.dir.clone();
                let input = project.clone();
                let generated = blocking(move || {
                    generate(&dir, &input, &tools, &|_| {}).map_err(|e| e.to_string())
                })
                .await??;
                cached.roads = Some(Arc::new(generated));
            }
            cached.roads_loaded = true;
        }
        Ok(MapData {
            project,
            heightmap: Arc::clone(&cached.heightmap),
            surfaces: cached.surfaces.clone(),
            imagery: cached.imagery.clone(),
            roads: cached.roads.clone(),
        })
    }

    #[tool(
        description = "The project (naves.toml) as JSON: name, area, scales, grid sizes and sources."
    )]
    async fn project_get(&self) -> Result<CallToolResult, McpError> {
        let dir = self.dir.clone();
        let loaded = blocking(move || Project::load(&dir).map_err(|e| e.to_string())).await;
        Ok(match loaded.and_then(|r| r) {
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
        let _writing = self.writes.lock().await;
        let dir = self.dir.clone();
        let result = blocking(move || -> Result<Project, String> {
            let project = Project::load(&dir).map_err(|e| e.to_string())?;
            let updated = project
                .with_value(&params.path, params.value)
                .map_err(|e| e.to_string())?;
            updated.save(&dir).map_err(|e| e.to_string())?;
            Ok(updated)
        })
        .await;
        Ok(match result.and_then(|r| r) {
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
        } = match self.map(wants_surfaces, wants_imagery, wants_roads).await {
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
        description = "The numbers of the generated road network: kilometres per surface, pieces, stretches and how many are not closed, the median distance of the pieces from the OpenStreetMap line, the steepest grade, the deepest cut and highest fill, the length that vehicles cannot drive, the largest distance of a piece from the ground and the length of paths that are too steep to walk. Generates the roads (armake2 and the DayZ Server install are needed). A project without OpenStreetMap files has empty numbers."
    )]
    async fn roads_stats(&self) -> Result<CallToolResult, McpError> {
        let map = match self.map(false, false, true).await {
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
        description = "What is wrong with the generated roads: a list of violations, each with its rule, a position in map metres and a message. Optional rule (road_not_drivable, road_misses_terrain, path_too_steep, road_grade, road_gap, road_in_water, road_conflict, earthworks_capped, orphaned_edit, join_without_junction, tunnel_omitted, junction_too_steep, road_far_from_ground, road_over_water) keeps only that rule. Generates the roads."
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
        let map = match self.map(false, false, true).await {
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
        let dir = self.dir.clone();
        let result = blocking(move || -> Result<serde_json::Value, String> {
            let project = Project::load(&dir).map_err(|e| e.to_string())?;
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
        let _writing = self.writes.lock().await;
        let dir = self.dir.clone();
        let result = blocking(move || -> Result<serde_json::Value, String> {
            let sources = catalog(user_catalog_path().as_deref()).map_err(|e| e.to_string())?;
            let source = find_source(&sources, &params.id).map_err(|e| e.to_string())?;
            let warnings = Mutex::new(Vec::new());
            let files = fetch(
                &dir,
                source,
                &FetchRequest {
                    from: params.from,
                    to: params.to,
                },
                &RealHttp::new(),
                &|line| {
                    if let (true, Ok(mut warnings)) = (line.starts_with("warning"), warnings.lock())
                    {
                        warnings.push(line.to_string());
                    }
                },
            )
            .map_err(|e| e.to_string())?;
            Ok(json!({
                "files": files.iter().map(|f| json!({
                    "path": f.path,
                    "attribution": f.attribution,
                    "licence": f.licence,
                    "non_commercial": f.non_commercial,
                })).collect::<Vec<_>>(),
                "warnings": warnings.into_inner().unwrap_or_default(),
            }))
        })
        .await;
        Ok(match result.and_then(|r| r) {
            Ok(value) => json_text(&value),
            Err(e) => failure(e),
        })
    }

    #[tool(
        description = "Build the mod and the server mission of the project (takes a while). Returns the mod and mission folders, warnings and the progress lines. Needs armake2, DayZ Tools and Proton, see the manual."
    )]
    async fn build(&self) -> Result<CallToolResult, McpError> {
        let _writing = self.writes.lock().await;
        let dir = self.dir.clone();
        let locate = Arc::clone(&self.locate_tools);
        let result = blocking(move || {
            let tools = locate()?;
            let lines = Mutex::new(Vec::new());
            let out = build(&dir, &tools, &|line| {
                if let Ok(mut lines) = lines.lock() {
                    lines.push(line.to_string());
                }
            })
            .map_err(|e| e.to_string())?;
            let progress = lines.into_inner().unwrap_or_default();
            Ok::<_, String>((out, progress))
        })
        .await;
        Ok(match result.and_then(|r| r) {
            Ok((out, progress)) => json_text(&json!({
                "mod_dir": out.mod_dir,
                "mission_dir": out.mission_dir,
                "warnings": out.warnings,
                "progress": progress,
            })),
            Err(e) => failure(e),
        })
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for NavesMcp {}

/// Serves the project in `dir` over standard input and output until the client disconnects.
///
/// # Errors
///
/// The transport fails or the server stops with an error.
pub async fn serve(dir: &Path) -> anyhow::Result<()> {
    let locate: ToolLocator =
        Arc::new(|| crate::user_prefs::locate_tools().map_err(|e| e.to_string()));
    let service = NavesMcp::new(dir, locate)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| anyhow::anyhow!("cannot start the MCP server: {e}"))?;
    service.waiting().await?;
    Ok(())
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the MCP tools ask for: the project, the map data, imagery and the build. [`Host`] is the seam between
//! the tools and where the data lives; [`StandaloneHost`] reads and writes the project folder of `naves mcp`.
//! All errors are the message text that the tools hand to the client.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use naves_core::dem::Dem;
use naves_core::edits::{EditsError, WayEdits};
use naves_core::heightmap::{Heightmap, build_heightmap};
use naves_core::imagery::catalog::{catalog, find_source, user_catalog_path};
use naves_core::imagery::{LoadedImagery, load_imagery};
use naves_core::project::{ImageryFile, Project};
use naves_core::surface::{SurfaceGrid, project_surfaces};
use naves_export::build::BuildOutput;
use naves_export::generate::{Generated, Source, generate, read_source};
use naves_export::imagery_fetch::{FetchRequest, RealHttp, fetch};
use naves_export::tools::Tools;

/// Finds the build tools when they are needed.
pub type ToolLocator = Arc<dyn Fn() -> Result<Tools, String> + Send + Sync>;

/// What a caller of [`Host::map`] needs besides the heightmap.
#[derive(Debug, Clone, Copy, Default)]
pub struct MapNeeds {
    /// The ground surfaces.
    pub surfaces: bool,
    /// The merged imagery.
    pub imagery: bool,
    /// The generated roads.
    pub roads: bool,
}

/// What [`Host::map`] hands out.
pub struct MapData {
    /// The project the data was made from.
    pub project: Project,
    /// The heightmap.
    pub heightmap: Arc<Heightmap>,
    /// The ground surfaces, when asked for.
    pub surfaces: Option<Arc<SurfaceGrid>>,
    /// The imagery, when asked for and the project lists some.
    pub imagery: Option<Arc<LoadedImagery>>,
    /// The generated roads, when asked for and the project lists an OpenStreetMap file.
    pub roads: Option<Arc<Generated>>,
}

/// Where the tools get the project and its data from.
pub enum Host {
    /// The project folder given to `naves mcp`.
    Standalone(StandaloneHost),
}

/// What the server tells its clients about coordinates, ids and edits.
const STANDALONE_INSTRUCTIONS: &str = "Coordinates are map metres from the south-west corner of the map: x \
east, y north. The tools that take a rectangle (render_topdown, roads_list) and the road tools use them. \
Look at the map with render_topdown, and find roads with roads_list. Road ids are strings: \"812\" for a way \
from OpenStreetMap, \"u3\" for a way drawn here. Road edits are written at once to edits/ways.toml in the \
project; there is no undo in this server. A build is needed to see the edits in the game.";

impl Host {
    /// The text the server sends to its clients on initialize.
    #[must_use]
    pub fn instructions(&self) -> &'static str {
        match self {
            Self::Standalone(_) => STANDALONE_INSTRUCTIONS,
        }
    }

    /// The saved project.
    ///
    /// # Errors
    ///
    /// The project cannot be loaded.
    pub async fn project(&self) -> Result<Project, String> {
        match self {
            Self::Standalone(host) => host.project().await,
        }
    }

    /// Sets one field of the project, saves it and returns the new project.
    ///
    /// # Errors
    ///
    /// The path or value is invalid, or the project cannot be loaded or saved.
    pub async fn set_project_value(
        &self,
        path: String,
        value: serde_json::Value,
    ) -> Result<Project, String> {
        match self {
            Self::Standalone(host) => host.set_project_value(path, value).await,
        }
    }

    /// The heightmap and what `needs` asks for, of the saved project.
    ///
    /// # Errors
    ///
    /// The data cannot be made.
    pub async fn map(&self, needs: MapNeeds) -> Result<MapData, String> {
        match self {
            Self::Standalone(host) => host.map(needs).await,
        }
    }

    /// The OpenStreetMap extract of the saved project with the current road edits applied.
    ///
    /// # Errors
    ///
    /// The project, the extract or `edits/ways.toml` cannot be read; the message names the file.
    pub async fn source(&self) -> Result<Arc<Source>, String> {
        match self {
            Self::Standalone(host) => host.source().await,
        }
    }

    /// Changes the road edits: runs `op` on a copy of the edits of the saved project and, when it succeeds,
    /// saves them and returns what `op` returned with the source layer that has them applied. Nothing is
    /// written when `op` or the saving fails. Calls are made one at a time, so none is lost to another.
    ///
    /// # Errors
    ///
    /// The source cannot be read, `op` refuses the edit (the message starts with "The edit cannot be made: ")
    /// or the edits cannot be saved.
    pub async fn edit_ways<T: Send + 'static>(
        &self,
        op: impl FnOnce(&mut WayEdits, &Source) -> Result<T, EditsError> + Send + 'static,
    ) -> Result<(T, Arc<Source>), String> {
        match self {
            Self::Standalone(host) => host.edit_ways(op).await,
        }
    }

    /// Fetches imagery from a catalogue source; returns the files and the warning lines.
    ///
    /// # Errors
    ///
    /// The source is unknown or the fetch fails.
    pub async fn fetch_imagery(
        &self,
        id: String,
        request: FetchRequest,
    ) -> Result<(Vec<ImageryFile>, Vec<String>), String> {
        match self {
            Self::Standalone(host) => host.fetch_imagery(id, request).await,
        }
    }

    /// Builds the mod and the mission; returns the output and the progress lines.
    ///
    /// # Errors
    ///
    /// The tools are missing or the build fails.
    pub async fn build(&self) -> Result<(BuildOutput, Vec<String>), String> {
        match self {
            Self::Standalone(host) => host.build().await,
        }
    }
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
    /// The road edits the `roads` were generated with; they are generated again when the file differs.
    roads_edits: Option<WayEdits>,
}

type CachedHeightmap = Option<Cached>;

/// The project folder of `naves mcp <dir>`, with the cache of what was computed from it.
pub struct StandaloneHost {
    dir: PathBuf,
    locate_tools: ToolLocator,
    heightmap: tokio::sync::Mutex<CachedHeightmap>,
    /// The source layer and the project it was read for.
    source: tokio::sync::Mutex<Option<(Project, Arc<Source>)>>,
    /// Held by what changes the project or its build folder, so that these run one at a time.
    writes: tokio::sync::Mutex<()>,
}

/// Runs blocking work off the async threads; a panic or a cancelled task becomes an error message.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("internal error: {e}"))
}

/// The road edits of the project folder; an error names `edits/ways.toml`.
fn load_edits(dir: &Path) -> Result<WayEdits, String> {
    WayEdits::load(dir).map_err(|e| match e {
        // an I/O error already names the file; the others do not
        EditsError::Io { .. } => e.to_string(),
        _ => format!("{}: {e}", dir.join("edits/ways.toml").display()),
    })
}

fn compute_heightmap(dir: &Path, project: &Project) -> Result<Heightmap, String> {
    if project.sources.dem.is_empty() {
        return Err("the project has no elevation data: add DEM files to sources.dem".to_string());
    }
    let files: Vec<PathBuf> = project.sources.dem.iter().map(|p| dir.join(p)).collect();
    let dem = Dem::open(&files).map_err(|e| e.to_string())?;
    build_heightmap(project, &dem).map_err(|e| e.to_string())
}

impl StandaloneHost {
    /// A host for the project in `dir`; `locate_tools` runs when a build starts.
    #[must_use]
    pub fn new(dir: &Path, locate_tools: ToolLocator) -> Self {
        Self {
            dir: dir.to_path_buf(),
            locate_tools,
            heightmap: tokio::sync::Mutex::new(None),
            source: tokio::sync::Mutex::new(None),
            writes: tokio::sync::Mutex::new(()),
        }
    }

    async fn project(&self) -> Result<Project, String> {
        let dir = self.dir.clone();
        blocking(move || Project::load(&dir).map_err(|e| e.to_string())).await?
    }

    async fn set_project_value(
        &self,
        path: String,
        value: serde_json::Value,
    ) -> Result<Project, String> {
        let _writing = self.writes.lock().await;
        let dir = self.dir.clone();
        blocking(move || -> Result<Project, String> {
            let project = Project::load(&dir).map_err(|e| e.to_string())?;
            let updated = project
                .with_value(&path, value)
                .map_err(|e| e.to_string())?;
            updated.save(&dir).map_err(|e| e.to_string())?;
            Ok(updated)
        })
        .await?
    }

    /// The heightmap and, when asked for, the ground surfaces, the imagery and the roads of the saved project.
    /// All are computed again only when the project file has changed; the rest is only made when asked for.
    async fn map(&self, needs: MapNeeds) -> Result<MapData, String> {
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
                roads_edits: None,
            });
        }
        let Some(cached) = cache.as_mut() else {
            return Err("internal error: no cached map".to_string());
        };
        if needs.surfaces && cached.surfaces.is_none() {
            let dir = self.dir.clone();
            let hm = Arc::clone(&cached.heightmap);
            let input = project.clone();
            let grid =
                blocking(move || project_surfaces(&input, &dir, &hm).map_err(|e| e.to_string()))
                    .await??;
            cached.surfaces = Some(Arc::new(grid));
        }
        if needs.imagery && !cached.imagery_loaded {
            let dir = self.dir.clone();
            let input = project.clone();
            let loaded =
                blocking(move || load_imagery(&input, &dir, &|_| {}).map_err(|e| e.to_string()))
                    .await??;
            cached.imagery = loaded.map(Arc::new);
            cached.imagery_loaded = true;
        }
        if needs.roads && !project.sources.osm.is_empty() {
            let dir = self.dir.clone();
            let edits = blocking(move || load_edits(&dir)).await??;
            if !cached.roads_loaded || cached.roads_edits.as_ref() != Some(&edits) {
                let tools =
                    (self.locate_tools)().map_err(|e| format!("Roads need dayzmake: {e}"))?;
                let dir = self.dir.clone();
                let input = project.clone();
                let generated = blocking(move || {
                    generate(&dir, &input, &tools, &|_| {}).map_err(|e| e.to_string())
                })
                .await??;
                cached.roads = Some(Arc::new(generated));
                // the edits read before generating: a change made meanwhile is noticed by the next call
                cached.roads_edits = Some(edits);
            }
        }
        if needs.roads {
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

    /// The source layer. The extract is read again only when the project file has changed; the edits are
    /// applied again only when `edits/ways.toml` differs from what the cached source was made with.
    async fn source(&self) -> Result<Arc<Source>, String> {
        Ok(self.source_of_project().await?.1)
    }

    async fn source_of_project(&self) -> Result<(Project, Arc<Source>), String> {
        let mut cache = self.source.lock().await;
        let dir = self.dir.clone();
        let cached = cache.clone();
        let (project, source) = blocking(move || -> Result<(Project, Arc<Source>), String> {
            let project = Project::load(&dir).map_err(|e| e.to_string())?;
            if let Some((cached_project, source)) = cached
                && cached_project == project
            {
                let edits = load_edits(&dir)?;
                if edits == source.edits {
                    return Ok((project, source));
                }
                let mut next = (*source).clone();
                next.edits = edits;
                next.reapply();
                return Ok((project, Arc::new(next)));
            }
            let source = read_source(&dir, &project).map_err(|e| e.to_string())?;
            Ok((project, Arc::new(source)))
        })
        .await??;
        *cache = Some((project.clone(), Arc::clone(&source)));
        Ok((project, source))
    }

    async fn edit_ways<T: Send + 'static>(
        &self,
        op: impl FnOnce(&mut WayEdits, &Source) -> Result<T, EditsError> + Send + 'static,
    ) -> Result<(T, Arc<Source>), String> {
        // held from the read to the write, so that parallel edits do not lose each other
        let _writing = self.writes.lock().await;
        let (project, source) = self.source_of_project().await?;
        let dir = self.dir.clone();
        let (value, next) = blocking(move || -> Result<(T, Arc<Source>), String> {
            let mut edits = source.edits.clone();
            let value =
                op(&mut edits, &source).map_err(|e| format!("The edit cannot be made: {e}"))?;
            edits
                .save(&dir)
                .map_err(|e| format!("Could not save the edit: {e}"))?;
            let mut next = (*source).clone();
            next.edits = edits;
            next.reapply();
            Ok((value, Arc::new(next)))
        })
        .await??;
        *self.source.lock().await = Some((project, Arc::clone(&next)));
        Ok((value, next))
    }

    async fn fetch_imagery(
        &self,
        id: String,
        request: FetchRequest,
    ) -> Result<(Vec<ImageryFile>, Vec<String>), String> {
        let _writing = self.writes.lock().await;
        let dir = self.dir.clone();
        blocking(move || {
            let sources = catalog(user_catalog_path().as_deref()).map_err(|e| e.to_string())?;
            let source = find_source(&sources, &id).map_err(|e| e.to_string())?;
            let warnings = Mutex::new(Vec::new());
            let files = fetch(&dir, source, &request, &RealHttp::new(), &|line| {
                if let (true, Ok(mut warnings)) = (line.starts_with("warning"), warnings.lock()) {
                    warnings.push(line.to_string());
                }
            })
            .map_err(|e| e.to_string())?;
            Ok::<_, String>((files, warnings.into_inner().unwrap_or_default()))
        })
        .await?
    }

    async fn build(&self) -> Result<(BuildOutput, Vec<String>), String> {
        let _writing = self.writes.lock().await;
        let dir = self.dir.clone();
        let locate = Arc::clone(&self.locate_tools);
        blocking(move || {
            let tools = locate()?;
            let lines = Mutex::new(Vec::new());
            let out = crate::cli::build_world(&dir, &tools, &|line| {
                if let Ok(mut lines) = lines.lock() {
                    lines.push(line.to_string());
                }
            })
            .map_err(|e| e.to_string())?;
            let progress = lines.into_inner().unwrap_or_default();
            Ok::<_, String>((out, progress))
        })
        .await?
    }
}

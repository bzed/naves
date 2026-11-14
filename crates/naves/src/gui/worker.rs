// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Background jobs of the GUI: loading the heightmap for the map view and running a build.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread::JoinHandle;

use naves_core::dem::Dem;
use naves_core::heightmap::{Heightmap, build_heightmap};
use naves_core::imagery::{LoadedImagery, load_imagery};
use naves_core::project::Project;
use naves_core::surface::{SurfaceGrid, project_surfaces};
use naves_export::build::{BuildOutput, build};
use naves_export::generate::{Generated, generate};
use naves_export::tools::Tools;

/// What a build reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildMsg {
    /// A progress line.
    Progress(String),
    /// The build finished.
    Done(BuildOutput),
    /// The build failed.
    Failed(String),
}

/// A build running on its own thread.
pub struct BuildWorker {
    rx: Receiver<BuildMsg>,
    handle: Option<JoinHandle<()>>,
    finished: bool,
}

impl BuildWorker {
    /// Starts building `dir`. `tools` is the result of locating the tools; an error is reported as a
    /// failed build.
    pub fn start(dir: &Path, tools: Result<Tools, String>) -> Self {
        let (tx, rx) = channel();
        let dir: PathBuf = dir.to_path_buf();
        let handle = std::thread::spawn(move || {
            // A closed channel means the window is gone; there is nobody left to tell.
            let send = |msg: BuildMsg| tx.send(msg).is_ok();
            let tools = match tools {
                Ok(tools) => tools,
                Err(error) => {
                    send(BuildMsg::Failed(error));
                    return;
                }
            };
            let result = build(&dir, &tools, &|line| {
                send(BuildMsg::Progress(line.to_string()));
            });
            send(match result {
                Ok(out) => BuildMsg::Done(out),
                Err(error) => BuildMsg::Failed(error.to_string()),
            });
        });
        Self {
            rx,
            handle: Some(handle),
            finished: false,
        }
    }

    /// Collects the messages that arrived since the last call.
    pub fn poll(&mut self) -> Vec<BuildMsg> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    if matches!(msg, BuildMsg::Done(_) | BuildMsg::Failed(_)) {
                        self.finished = true;
                    }
                    out.push(msg);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    break;
                }
            }
        }
        if self.finished
            && let Some(handle) = self.handle.take()
        {
            handle.join().ok(); // the thread has sent its last message; a panic was reported as Disconnected
        }
        out
    }

    /// Whether the build has ended (successfully or not).
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Blocks until the next message or the end (for tests and shutdown).
    #[cfg(all(test, unix))]
    pub fn wait(&mut self, timeout: std::time::Duration) -> Vec<BuildMsg> {
        let mut all = Vec::new();
        let deadline = std::time::Instant::now() + timeout;
        while !self.finished && std::time::Instant::now() < deadline {
            if let Ok(msg) = self.rx.recv_timeout(std::time::Duration::from_millis(50)) {
                if matches!(msg, BuildMsg::Done(_) | BuildMsg::Failed(_)) {
                    self.finished = true;
                }
                all.push(msg);
            }
        }
        all
    }
}

/// What the map view draws: the heightmap and the ground surfaces decided from it and the sources.
pub struct MapData {
    /// The heights.
    pub heightmap: Heightmap,
    /// The ground class of every map cell.
    pub surfaces: SurfaceGrid,
    /// The merged imagery, if the project lists any.
    pub imagery: Option<LoadedImagery>,
}

/// The state of the heightmap behind the map view.
pub enum HeightmapState {
    /// The project has no DEM files yet.
    NoDem,
    /// Being computed.
    Loading,
    /// Ready to draw.
    Ready(std::sync::Arc<MapData>),
    /// Could not be computed; the text says why.
    Failed(String),
}

/// A heightmap computation on its own thread.
pub struct HeightmapJob {
    rx: Receiver<Result<MapData, String>>,
}

impl HeightmapJob {
    /// Computes the heightmap and the ground surfaces of the project in `dir` as saved on disk, in one go.
    pub fn start(dir: &Path) -> Self {
        let (tx, rx) = channel();
        let dir = dir.to_path_buf();
        std::thread::spawn(move || {
            let result = (|| {
                let project = Project::load(&dir).map_err(|e| e.to_string())?;
                let files: Vec<PathBuf> = project.sources.dem.iter().map(|p| dir.join(p)).collect();
                let dem = Dem::open(&files).map_err(|e| e.to_string())?;
                let heightmap = build_heightmap(&project, &dem).map_err(|e| e.to_string())?;
                let surfaces =
                    project_surfaces(&project, &dir, &heightmap).map_err(|e| e.to_string())?;
                let imagery = load_imagery(&project, &dir, &|_| {}).map_err(|e| e.to_string())?;
                Ok(MapData {
                    heightmap,
                    surfaces,
                    imagery,
                })
            })();
            tx.send(result).ok(); // the window may be gone already
        });
        Self { rx }
    }

    /// The result, once it is there.
    pub fn poll(&self) -> Option<Result<MapData, String>> {
        self.rx.try_recv().ok()
    }
}

/// The state of the roads behind the road layers.
pub enum RoadsState {
    /// The project lists no OpenStreetMap file.
    NoOsm,
    /// Waiting for the map, or being generated.
    Generating,
    /// Ready to draw.
    Ready(std::sync::Arc<Generated>),
    /// The tools that the road pieces are read with cannot be found; the text says which.
    ToolsMissing(String),
    /// The roads could not be generated; the text says why.
    Failed(String),
}

/// What a road job reports.
pub enum RoadsMsg {
    /// The roads.
    Ready(Box<Generated>),
    /// The tools are missing.
    ToolsMissing(String),
    /// The generation failed.
    Failed(String),
}

/// The generation of the roads on its own thread.
pub struct RoadJob {
    rx: Receiver<RoadsMsg>,
}

impl RoadJob {
    /// Generates the roads of the project in `dir` as saved on disk. `tools` is the result of locating them.
    pub fn start(dir: &Path, tools: Result<Tools, String>) -> Self {
        let (tx, rx) = channel();
        let dir = dir.to_path_buf();
        std::thread::spawn(move || {
            let msg = match tools {
                Err(error) => RoadsMsg::ToolsMissing(error),
                Ok(tools) => match Project::load(&dir) {
                    Err(error) => RoadsMsg::Failed(error.to_string()),
                    Ok(project) => match generate(&dir, &project, &tools, &|_| {}) {
                        Ok(generated) => RoadsMsg::Ready(Box::new(generated)),
                        Err(error) => RoadsMsg::Failed(error.to_string()),
                    },
                },
            };
            tx.send(msg).ok(); // the window may be gone already
        });
        Self { rx }
    }

    /// The result, once it is there.
    pub fn poll(&self) -> Option<RoadsMsg> {
        self.rx.try_recv().ok()
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::time::Duration;

    use naves_export::testing::FakeEnv;

    use super::*;

    fn project(dir: &Path) {
        let mut p = Project::new("Test", 47.55, 12.975, 1500.0);
        p.grid.terrain_size = 256;
        p.grid.land_size = 32;
        p.sources.dem = vec!["dem/koenigssee.tif".into()];
        std::fs::create_dir_all(dir.join("dem")).unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dem/koenigssee.tif");
        std::fs::copy(fixture, dir.join("dem/koenigssee.tif")).unwrap();
        p.save(dir).unwrap();
    }

    #[test]
    fn a_build_reports_progress_then_the_result() {
        let root = tempfile::tempdir().unwrap();
        let fakes = FakeEnv::new(&root.path().join("fake")).unwrap();
        let proj = root.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        project(&proj);
        let mut worker = BuildWorker::start(&proj, Ok(fakes.tools()));
        let msgs = worker.wait(Duration::from_mins(1));
        assert!(
            msgs.iter().any(|m| matches!(m, BuildMsg::Progress(_))),
            "{msgs:?}"
        );
        assert!(matches!(msgs.last(), Some(BuildMsg::Done(_))), "{msgs:?}");
        assert!(worker.poll().is_empty() && worker.is_finished());
    }

    #[test]
    fn a_failed_build_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let fakes = FakeEnv::new(&root.path().join("fake")).unwrap();
        let proj = root.path().join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        Project::new("x", 47.55, 12.975, 1500.0)
            .save(&proj)
            .unwrap();
        let mut worker = BuildWorker::start(&proj, Ok(fakes.tools()));
        let msgs = worker.wait(Duration::from_mins(1));
        match msgs.last() {
            Some(BuildMsg::Failed(text)) => assert!(text.contains("sources.dem"), "{text}"),
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn missing_tools_are_a_failed_build() {
        let root = tempfile::tempdir().unwrap();
        let mut worker = BuildWorker::start(
            root.path(),
            Err("armake2 not found; set ARMAKE2".to_string()),
        );
        let msgs = worker.wait(Duration::from_secs(10));
        assert_eq!(
            msgs,
            vec![BuildMsg::Failed(
                "armake2 not found; set ARMAKE2".to_string()
            )]
        );
    }

    #[test]
    fn the_heightmap_job_delivers() {
        let root = tempfile::tempdir().unwrap();
        project(root.path());
        let job = HeightmapJob::start(root.path());
        let deadline = std::time::Instant::now() + Duration::from_mins(1);
        let result = loop {
            if let Some(r) = job.poll() {
                break r;
            }
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::yield_now();
        };
        assert_eq!(result.unwrap().heightmap.size, 256);
    }
}

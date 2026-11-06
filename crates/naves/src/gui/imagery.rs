// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Imagery in the editor: what can be fetched for the area, fetching in the background, the files of the
//! project in their order, files of your own, and what the merge found out about each one.
//!
//! Look: the project panel's, a heading per group, the licence of every file in words (never only a colour) and
//! the fetch buttons next to the sources they belong to.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread::JoinHandle;

use eframe::egui::{self, Color32, RichText};
use naves_core::heightmap::MapGeo;
use naves_core::imagery::MergeReport;
use naves_core::imagery::catalog::{
    CatalogSource, ImageryOption, WHOLE_MAP, catalog, find_source, options_for, user_catalog_path,
};
use naves_core::project::{ImageryFile, Project};
use naves_export::imagery_fetch::{FetchError, FetchRequest, Http, RealHttp, fetch};

use super::{BROWN, ERROR, GREEN, HeightmapState, Line, NavesApp};

/// A file of your own that is about to be added: all four fields are needed before it is accepted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnFile {
    /// The path relative to the project folder.
    pub path: String,
    /// The credit line.
    pub attribution: String,
    /// The licence in words.
    pub licence: String,
    /// The licence does not allow commercial use.
    pub non_commercial: bool,
}

/// The sources that have imagery for the area of `project`, best first. `user_file` is the user's own catalogue.
///
/// # Errors
///
/// The reason, in words, if the area or the catalogue cannot be read.
pub fn options_for_project(
    project: &Project,
    user_file: Option<&Path>,
) -> Result<Vec<ImageryOption>, String> {
    let window = MapGeo::new(project)
        .and_then(|geo| geo.window(0.0))
        .map_err(|e| e.to_string())?;
    let sources = catalog(user_file).map_err(|e| e.to_string())?;
    Ok(options_for(&sources, window))
}

/// What a source offers, in a line: how much of the map, the pixel size, the licence, and the words
/// "non-commercial" where it does not allow commercial use.
pub fn option_line(option: &ImageryOption) -> String {
    let covers = if option.coverage_share >= WHOLE_MAP {
        "whole map".to_string()
    } else {
        format!(
            "part of the map (about {:.0} %)",
            option.coverage_share * 100.0
        )
    };
    let pixel = if option.source.native_m >= 1.0 {
        format!("{:.0} m", option.source.native_m)
    } else {
        format!("{} m", option.source.native_m)
    };
    let mark = if option.source.non_commercial {
        ", non-commercial"
    } else {
        ""
    };
    format!("{covers}, {pixel}, {}{mark}", option.source.licence)
}

/// Checks a file of your own the way the project file does; the error names the field.
///
/// # Errors
///
/// The reason, in words, why the file is not accepted.
pub fn validate_own(project: &Project, own: &OwnFile) -> Result<ImageryFile, String> {
    let file = ImageryFile {
        path: PathBuf::from(own.path.trim()),
        attribution: own.attribution.trim().to_string(),
        licence: own.licence.trim().to_string(),
        non_commercial: own.non_commercial,
    };
    let mut files = project.sources.imagery.clone();
    files.push(file.clone());
    let value = serde_json::to_value(&files).map_err(|e| e.to_string())?;
    project
        .with_value("sources.imagery", value)
        .map_err(|e| e.to_string())?;
    Ok(file)
}

/// One line per file about what the merge found out.
pub fn report_lines(report: &MergeReport) -> Vec<String> {
    report
        .sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let colours = if report.reference == Some(index) {
                "its colours are kept".to_string()
            } else {
                let [r, g, b] = source.median_gain;
                format!("colour correction x{r:.2}, x{g:.2}, x{b:.2}")
            };
            format!(
                "{}: covers {:.0} % of the map, overlaps other images on {:.0} % of it, {colours}",
                source.label,
                source.coverage * 100.0,
                source.overlap * 100.0
            )
        })
        .collect()
}

/// What a fetch reports back.
#[derive(Debug, Clone, PartialEq)]
pub enum FetchMsg {
    /// A progress line.
    Progress(String),
    /// The files that were added.
    Done(Vec<ImageryFile>),
    /// The fetch failed.
    Failed(String),
    /// The user stopped it; no file was written.
    Cancelled,
}

/// A fetch running on its own thread.
pub struct FetchJob {
    rx: Receiver<FetchMsg>,
    cancel: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    finished: bool,
}

/// The network with a stop button: once cancelled, every request and every write fails.
struct Cancellable {
    inner: Arc<dyn Http + Send + Sync>,
    cancel: Arc<AtomicBool>,
}

fn cancelled() -> FetchError {
    FetchError::Other("cancelled".to_string())
}

/// A writer that fails once the fetch is cancelled, so that a download stops in the middle.
struct StopWriter<'a> {
    inner: &'a mut dyn Write,
    cancel: &'a AtomicBool,
}

impl Write for StopWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(std::io::Error::other("cancelled"));
        }
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Http for Cancellable {
    fn get(&self, url: &str, max_bytes: u64, out: &mut dyn Write) -> Result<(), FetchError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        let mut guarded = StopWriter {
            inner: out,
            cancel: &self.cancel,
        };
        self.inner.get(url, max_bytes, &mut guarded)
    }

    fn post_json(&self, url: &str, body: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        self.inner.post_json(url, body, max_bytes)
    }
}

impl FetchJob {
    /// Starts fetching `source` into the project in `dir`.
    pub fn start(
        dir: &Path,
        source: CatalogSource,
        request: FetchRequest,
        http: Arc<dyn Http + Send + Sync>,
    ) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let dir = dir.to_path_buf();
        let stop = Arc::clone(&cancel);
        let handle = std::thread::spawn(move || {
            // a closed channel means the window is gone; there is nobody left to tell
            let send = |msg: FetchMsg| tx.send(msg).is_ok();
            let network = Cancellable {
                inner: http,
                cancel: Arc::clone(&stop),
            };
            let result = fetch(&dir, &source, &request, &network, &|line| {
                send(FetchMsg::Progress(line.to_string()));
            });
            send(match result {
                Ok(files) => FetchMsg::Done(files),
                Err(_) if stop.load(Ordering::SeqCst) => FetchMsg::Cancelled,
                Err(error) => FetchMsg::Failed(error.to_string()),
            });
        });
        Self {
            rx,
            cancel,
            handle: Some(handle),
            finished: false,
        }
    }

    /// Stops the fetch: no further request is made and nothing is written.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// The messages that arrived since the last call.
    pub fn poll(&mut self) -> Vec<FetchMsg> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    if !matches!(msg, FetchMsg::Progress(_)) {
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

    /// Whether the fetch has ended, successfully or not.
    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

/// The real network for the editor.
pub fn real_http() -> Arc<dyn Http + Send + Sync> {
    Arc::new(RealHttp::new())
}

/// The user's own catalogue file, if the configuration folder is known.
pub fn user_catalog() -> Option<PathBuf> {
    user_catalog_path()
}

impl NavesApp {
    /// The same editor with another network (for tests).
    #[cfg(test)]
    pub fn with_http(mut self, http: Arc<dyn Http + Send + Sync>) -> Self {
        self.http = http;
        self
    }

    /// Works out again which sources have imagery for the area.
    pub(super) fn refresh_imagery_options(&mut self) {
        match options_for_project(&self.form.project, self.user_catalog.as_deref()) {
            Ok(options) => {
                self.imagery_options = options;
                self.imagery_options_error = None;
            }
            Err(error) => {
                self.imagery_options.clear();
                self.imagery_options_error = Some(error);
            }
        }
    }

    /// Whether a fetch is running.
    pub fn fetch_running(&self) -> bool {
        self.fetch.is_some()
    }

    /// Starts fetching the source `id` for the project, one fetch at a time.
    ///
    /// # Errors
    ///
    /// The reason, in words: a fetch is already running, or no source has that id (the message lists them).
    pub fn start_fetch(&mut self, id: &str, request: FetchRequest) -> Result<(), String> {
        if self.fetch.is_some() {
            return Err("a fetch is already running; wait for it or cancel it".to_string());
        }
        let sources = catalog(self.user_catalog.as_deref()).map_err(|e| e.to_string())?;
        let source = find_source(&sources, id)
            .map_err(|e| e.to_string())?
            .clone();
        self.log.push(Line::Info(format!(
            "Fetching {} ({}){}",
            source.name,
            source.licence,
            if source.non_commercial {
                ": non-commercial, the finished map will say so"
            } else {
                ""
            }
        )));
        self.fetch = Some(FetchJob::start(
            self.form.dir(),
            source,
            request,
            Arc::clone(&self.http),
        ));
        Ok(())
    }

    /// Stops the running fetch; nothing it had not finished is written.
    pub fn cancel_fetch(&self) {
        if let Some(job) = &self.fetch {
            job.cancel();
        }
    }

    /// Takes the messages of the fetch into the log; when it is done, the project follows the file.
    pub(super) fn poll_fetch(&mut self) {
        let Some(job) = self.fetch.as_mut() else {
            return;
        };
        let messages = job.poll();
        if job.is_finished() {
            self.fetch = None;
        }
        for message in messages {
            match message {
                FetchMsg::Progress(line) => self.log.push(if line.starts_with("warning") {
                    Line::Warning(line)
                } else {
                    Line::Info(line)
                }),
                FetchMsg::Done(files) => {
                    for file in &files {
                        self.log.push(Line::Result(format!(
                            "Added {} ({})",
                            file.path.display(),
                            file.licence
                        )));
                    }
                    // the fetch saved the list: the form follows the file, so the next edit keeps it
                    if let Ok(saved) = Project::load(self.form.dir()) {
                        self.form.project.sources.imagery = saved.sources.imagery;
                    }
                    self.settings_changed();
                }
                FetchMsg::Failed(error) => self.log.push(Line::Error(error)),
                FetchMsg::Cancelled => self.log.push(Line::Info(
                    "The fetch was cancelled; no file was written".to_string(),
                )),
            }
        }
    }

    /// Moves the file at `index` up or down the list (the order decides among equally sharp images).
    pub fn move_imagery(&mut self, index: usize, up: bool) {
        let files = &mut self.form.project.sources.imagery;
        let other = if up {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        let Some(other) = other.filter(|&o| o < files.len() && index < files.len()) else {
            return;
        };
        files.swap(index, other);
        self.settings_changed();
    }

    /// Takes the file at `index` off the list. The file itself stays in the project folder.
    pub fn remove_imagery(&mut self, index: usize) {
        if index >= self.form.project.sources.imagery.len() {
            return;
        }
        let removed = self.form.project.sources.imagery.remove(index);
        self.log.push(Line::Info(format!(
            "{} is off the list; the file stays in the project folder",
            removed.path.display()
        )));
        self.settings_changed();
    }

    /// Adds a file of your own, once it has a credit and a licence.
    ///
    /// # Errors
    ///
    /// The reason, in words, naming the field that is wrong; nothing is saved then.
    pub fn add_own_imagery(&mut self, own: &OwnFile) -> Result<(), String> {
        let file = validate_own(&self.form.project, own)?;
        self.form.project.sources.imagery.push(file);
        self.settings_changed();
        Ok(())
    }

    /// One line per file about what the merge found out, once the map is loaded.
    pub fn imagery_report_lines(&self) -> Vec<String> {
        match &self.heightmap {
            HeightmapState::Ready(data) => data
                .imagery
                .as_ref()
                .map(|loaded| report_lines(&loaded.report))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// The imagery part of the project panel.
    pub(super) fn imagery_section(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Satellite imagery files").strong());
        self.imagery_files_ui(ui);
        ui.add_space(8.0);
        self.imagery_fetch_ui(ui);
        ui.add_space(8.0);
        self.imagery_own_ui(ui);
    }

    fn imagery_files_ui(&mut self, ui: &mut egui::Ui) {
        if self.form.project.sources.imagery.is_empty() {
            ui.label(
                "None. The satellite image is painted from the ground classes. Fetch imagery below, or add \
                 a file of your own.",
            );
            return;
        }
        let used: Option<Vec<ImageryFile>> = match &self.heightmap {
            HeightmapState::Ready(data) => data.imagery.as_ref().map(|i| i.used.clone()),
            _ => None,
        };
        let files = self.form.project.sources.imagery.clone();
        let last = files.len().saturating_sub(1);
        let mut action: Option<(usize, bool, bool)> = None; // (index, up, remove)
        for (index, file) in files.iter().enumerate() {
            ui.label(file.path.display().to_string());
            let licence = if file.non_commercial {
                format!("{}, non-commercial", file.licence)
            } else {
                file.licence.clone()
            };
            ui.label(RichText::new(licence).small().color(BROWN));
            ui.horizontal(|ui| {
                if ui.add_enabled(index > 0, egui::Button::new("Up")).clicked() {
                    action = Some((index, true, false));
                }
                if ui
                    .add_enabled(index < last, egui::Button::new("Down"))
                    .clicked()
                {
                    action = Some((index, false, false));
                }
                if ui.button("Remove").clicked() {
                    action = Some((index, false, true));
                }
            });
        }
        for line in self.imagery_report_lines() {
            ui.label(RichText::new(line).small());
        }
        let relevant = used.as_deref().unwrap_or(&files);
        if relevant.iter().any(|f| f.non_commercial) {
            ui.add_space(4.0);
            ui.label(RichText::new("Non-commercial map").strong().color(BROWN));
        }
        ui.label(
            RichText::new(
                "The order decides among equally sharp images: the first is drawn on top. Remove only takes a \
                 file off the list; it stays in the project folder.",
            )
            .small(),
        );
        match action {
            Some((index, _, true)) => self.remove_imagery(index),
            Some((index, up, false)) => self.move_imagery(index, up),
            None => {}
        }
    }

    fn imagery_fetch_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Available for this area").strong());
        if let Some(error) = &self.imagery_options_error {
            ui.colored_label(ERROR, format!("The sources cannot be listed: {error}"));
            return;
        }
        let busy = self.fetch_running();
        let options = self.imagery_options.clone();
        let mut wanted: Option<String> = None;
        for option in &options {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&option.source.name).strong());
                let button =
                    egui::Button::new(RichText::new("Fetch").color(Color32::WHITE)).fill(GREEN);
                if ui.add_enabled(!busy, button).clicked() {
                    wanted = Some(option.source.id.clone());
                }
            });
            ui.label(RichText::new(option_line(option)).small());
        }
        ui.horizontal(|ui| {
            ui.label("Dates for scenes");
            ui.add(
                egui::TextEdit::singleline(&mut self.fetch_from)
                    .hint_text("2025-05-01")
                    .desired_width(78.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.fetch_to)
                    .hint_text("2025-09-30")
                    .desired_width(78.0),
            );
        });
        if let Some(id) = wanted {
            let request = FetchRequest {
                from: Some(self.fetch_from.trim().to_string()).filter(|s| !s.is_empty()),
                to: Some(self.fetch_to.trim().to_string()).filter(|s| !s.is_empty()),
            };
            if let Err(error) = self.start_fetch(&id, request) {
                self.log.push(Line::Error(error));
            }
        }
        if busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Fetching: see the log");
                if ui.button("Cancel").clicked() {
                    self.cancel_fetch();
                }
            });
        }
    }

    fn imagery_own_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Add your own file").show(ui, |ui| {
            let own = &mut self.own_imagery;
            ui.label("File, relative to the project folder");
            ui.text_edit_singleline(&mut own.path);
            ui.label("Credit");
            ui.text_edit_singleline(&mut own.attribution);
            ui.label("Licence");
            ui.text_edit_singleline(&mut own.licence);
            ui.checkbox(
                &mut own.non_commercial,
                "The licence does not allow commercial use",
            );
            if ui.button("Add file").clicked() {
                let draft = self.own_imagery.clone();
                match self.add_own_imagery(&draft) {
                    Ok(()) => {
                        self.own_imagery = OwnFile::default();
                        self.own_imagery_error = None;
                    }
                    Err(error) => self.own_imagery_error = Some(error),
                }
            }
            if let Some(error) = &self.own_imagery_error {
                ui.colored_label(ERROR, format!("Not added: {error}"));
            }
        });
    }
}

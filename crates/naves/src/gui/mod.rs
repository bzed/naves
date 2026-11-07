// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The graphical editor: project settings, a top-down map and the build.
//!
//! The widget code is thin; the logic lives in plain structs (`settings`, `map_view`, `worker`).
//! Look: the manual's palette. A pale green ground, ink text, village-green accents, and the map as the
//! one loud thing on the screen.

mod dock;
mod imagery;
mod layers;
mod layout;
mod levels;
mod map_view;
mod palette;
mod prefs_window;
mod problems;
mod rail;
mod selection;
mod settings;
mod shortcuts;
mod stack;
#[cfg(test)]
mod testing;
mod theme;
mod viewport;
mod worker;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use naves_core::imagery::catalog::ImageryOption;
use naves_core::prefs::{Preferences, ToolPaths};
use naves_core::project::{Area, Grid, ImageryFile, LAND_RATIO, Project, Scale};
use naves_core::render::Overlay;
use naves_export::build::BuildOutput;
use naves_export::imagery_fetch::Http;
use naves_export::tools::{ToolOrigin, Tools, tool_origins};

use imagery::{FetchJob, OwnFile};
use levels::LevelDef;
use settings::SettingsForm;
use stack::LayerPanel;
use viewport::{ViewEvent, ViewFrame, ViewportSet, default_views};
use worker::{BuildMsg, BuildWorker, HeightmapJob, HeightmapState};

use theme::{ERROR, GREEN};
const SETTINGS_WIDTH: f32 = 300.0;
const RIGHT_WIDTH: f32 = 230.0;
const BAND_HEIGHT: f32 = 3.0;
/// How many metres Frame selection shows when neither the selection nor the view says.
const FRAME_SPAN_M: f64 = 300.0;
const STATUS_HEIGHT: f32 = 26.0;

type ToolLocator = Box<dyn Fn() -> Result<Tools, String>>;

enum Line {
    Info(String),
    Warning(String),
    Error(String),
    Result(String),
}

/// The application state.
pub struct NavesApp {
    form: SettingsForm,
    views: ViewportSet,
    /// Counts up with every new `MapData` and every change of what the pictures show.
    generation: u64,
    /// The levels that are built, bottom first.
    levels: Vec<&'static LevelDef>,
    /// The base map and the active level.
    panel: LayerPanel,
    /// The left panel was opened by hand in a window that is too small for it.
    left_open: bool,
    /// The sheet of shortcuts (F1) is open.
    help_open: bool,
    /// What a click picked.
    selection: Option<selection::Selection>,
    /// The tab of the bottom dock.
    dock: &'static str,
    /// The tool in use.
    tool: shortcuts::ToolId,
    /// The item that was picked last.
    item: Option<String>,
    /// The search and the last used items of the palette.
    palette: palette::Palette,
    /// The keys in force.
    bindings: shortcuts::Bindings,
    /// What in the settings file's shortcuts could not be read.
    binding_notes: Vec<String>,
    /// The per-user settings.
    prefs: Preferences,
    /// Where they are saved; without a path nothing is saved (tests, and a person without a configuration folder).
    prefs_path: Option<PathBuf>,
    /// The settings file is damaged: this says why. Changes hold for the session and are not saved.
    prefs_error: Option<String>,
    /// The last save failed: this says why.
    prefs_save_error: Option<String>,
    /// The settings window, while it is open.
    prefs_window: Option<prefs_window::PrefsWindow>,
    /// Where the tools come from; replaced in tests.
    origins: fn(&ToolPaths) -> Vec<ToolOrigin>,
    /// What is wrong, from all sources.
    problems: problems::ProblemList,
    /// Warnings and errors of the last build; cleared when the next build starts.
    build_problems: Vec<problems::Problem>,
    /// Notes of the imagery merge; replaced with each new map.
    merge_problems: Vec<problems::Problem>,
    heightmap: HeightmapState,
    job: Option<HeightmapJob>,
    /// The settings changed while `job` was running, so its result is out of date.
    reload_wanted: bool,
    /// The project values the current heightmap depends on.
    heightmap_inputs: HeightmapInputs,
    /// What the views reported in the last frames (for tests).
    #[cfg(test)]
    seen_events: Vec<ViewEvent>,
    /// The rectangle the active view was given (for tests).
    #[cfg(test)]
    seen_rect: egui::Rect,
    /// The id of the palette row under the pointer, if any (for tests).
    #[cfg(test)]
    pub hovered_row: Option<String>,
    worker: Option<BuildWorker>,
    log: Vec<Line>,
    locate_tools: ToolLocator,
    /// A fetch of imagery, running in the background.
    fetch: Option<FetchJob>,
    /// The network that fetches use.
    http: Arc<dyn Http + Send + Sync>,
    /// The user's own catalogue of imagery sources.
    user_catalog: Option<PathBuf>,
    /// The sources that have imagery for the area of the project.
    imagery_options: Vec<ImageryOption>,
    imagery_options_error: Option<String>,
    /// The dates typed for a search for scenes.
    fetch_from: String,
    fetch_to: String,
    /// The file of your own that is being filled in, and why it was not added.
    own_imagery: OwnFile,
    own_imagery_error: Option<String>,
}

impl NavesApp {
    /// Opens the project in `dir`; `locate_tools` is called when a build starts.
    ///
    /// # Errors
    ///
    /// The project file cannot be loaded.
    pub fn open(
        dir: &Path,
        locate_tools: ToolLocator,
    ) -> Result<Self, naves_core::project::ProjectError> {
        let form = SettingsForm::load(dir)?;
        let inputs = heightmap_inputs(&form.project);
        let mut app = Self {
            form,
            views: ViewportSet::new(default_views()),
            generation: 0,
            levels: levels::BUILT.to_vec(),
            panel: LayerPanel::default(),
            left_open: false,
            help_open: false,
            selection: None,
            dock: "build",
            tool: shortcuts::ToolId::SELECT,
            item: None,
            palette: palette::Palette::default(),
            bindings: shortcuts::Bindings::default(),
            binding_notes: Vec::new(),
            prefs: Preferences::default(),
            prefs_path: None,
            prefs_error: None,
            prefs_save_error: None,
            prefs_window: None,
            origins: tool_origins,
            problems: problems::ProblemList::default(),
            build_problems: Vec::new(),
            merge_problems: Vec::new(),
            heightmap: HeightmapState::NoDem,
            job: None,
            reload_wanted: false,
            heightmap_inputs: inputs,
            #[cfg(test)]
            seen_events: Vec::new(),
            #[cfg(test)]
            seen_rect: egui::Rect::NOTHING,
            #[cfg(test)]
            hovered_row: None,
            worker: None,
            log: Vec::new(),
            locate_tools,
            fetch: None,
            http: imagery::real_http(),
            user_catalog: imagery::user_catalog(),
            imagery_options: Vec::new(),
            imagery_options_error: None,
            fetch_from: String::new(),
            fetch_to: String::new(),
            own_imagery: OwnFile::default(),
            own_imagery_error: None,
        };
        app.refresh_imagery_options();
        app.reload_heightmap();
        Ok(app)
    }

    /// Keeps the per-user settings in `path`: reads them now, applies them, and saves changes there. A damaged file is
    /// reported in the settings window and never overwritten.
    #[must_use]
    pub fn with_preferences(mut self, path: PathBuf) -> Self {
        match Preferences::load(&path) {
            Ok(prefs) => self.prefs = prefs,
            Err(error) => self.prefs_error = Some(error.to_string()),
        }
        self.prefs_path = Some(path);
        self.apply_prefs();
        self.panel = stack::LayerPanel::from_prefs(&self.prefs.map);
        self
    }

    /// Takes the shortcuts of the settings.
    fn apply_prefs(&mut self) {
        let (bindings, notes) = shortcuts::Bindings::new(&self.prefs.shortcuts);
        self.bindings = bindings;
        self.binding_notes = notes;
    }

    /// Saves the settings after a change. Nothing is written without a path or over a damaged file.
    fn save_prefs(&mut self) {
        self.prefs_save_error = None;
        let Some(path) = &self.prefs_path else {
            return;
        };
        if self.prefs_error.is_some() {
            return;
        }
        // keys of the file that this version does not know stay as they are
        let known: Vec<&str> = shortcuts::SHORTCUTS.iter().map(|s| s.id).collect();
        self.prefs
            .shortcuts
            .retain(|id, _| !known.contains(&id.as_str()));
        self.prefs.shortcuts.extend(self.bindings.overrides());
        if let Err(error) = self.prefs.save(path) {
            self.prefs_save_error = Some(format!("Not saved: {error}"));
        }
    }

    /// Replaces the lookup of the tools' origins (for tests).
    #[cfg(test)]
    fn with_tool_origins(mut self, origins: fn(&ToolPaths) -> Vec<ToolOrigin>) -> Self {
        self.origins = origins;
        self
    }

    /// The window title.
    pub fn title(&self) -> String {
        format!("Náves – {}", self.form.project.name)
    }

    fn reload_heightmap(&mut self) {
        self.views.reset_all();
        self.selection = None;
        if self.form.project.sources.dem.is_empty() {
            self.heightmap = HeightmapState::NoDem;
            self.job = None;
        } else {
            self.heightmap = HeightmapState::Loading;
            if self.job.is_some() {
                // One job at a time: the next one starts when this one ends, with what is saved by then.
                self.reload_wanted = true;
            } else {
                self.job = Some(HeightmapJob::start(self.form.dir()));
            }
        }
    }

    /// Saves changed settings; only what the heightmap depends on makes it load again.
    fn settings_changed(&mut self) {
        if self.form.commit() {
            // a setting can change how the map looks (the imagery brightness) without changing its data
            self.generation += 1;
            self.views.invalidate_all();
            self.refresh_imagery_options();
            let inputs = heightmap_inputs(&self.form.project);
            if inputs != self.heightmap_inputs {
                self.heightmap_inputs = inputs;
                self.reload_heightmap();
            }
        }
    }

    fn poll_background(&mut self) {
        self.poll_fetch();
        if let Some(result) = self.job.as_ref().and_then(HeightmapJob::poll) {
            self.job = None;
            if std::mem::take(&mut self.reload_wanted) {
                self.job = Some(HeightmapJob::start(self.form.dir()));
            } else {
                self.heightmap = match result {
                    Ok(data) => {
                        self.merge_problems.clear();
                        if let Some(imagery) = &data.imagery {
                            self.log
                                .extend(imagery.warnings.iter().cloned().map(Line::Warning));
                            self.merge_problems.extend(
                                imagery
                                    .warnings
                                    .iter()
                                    .cloned()
                                    .map(problems::Problem::note),
                            );
                        }
                        self.generation += 1;
                        HeightmapState::Ready(Arc::new(data))
                    }
                    Err(error) => HeightmapState::Failed(error),
                };
            }
        }
        if let Some(worker) = self.worker.as_mut() {
            for msg in worker.poll() {
                match msg {
                    BuildMsg::Progress(line) => self.log.push(Line::Info(line)),
                    BuildMsg::Failed(error) => {
                        self.build_problems
                            .push(problems::Problem::note(error.clone()));
                        self.log.push(Line::Error(error));
                    }
                    BuildMsg::Done(BuildOutput {
                        mod_dir,
                        mission_dir,
                        warnings,
                    }) => {
                        self.build_problems
                            .extend(warnings.iter().cloned().map(problems::Problem::note));
                        self.log.extend(warnings.into_iter().map(Line::Warning));
                        self.log.push(Line::Result(format!(
                            "Mod: {}\nMission: {}",
                            mod_dir.display(),
                            mission_dir.display()
                        )));
                    }
                }
            }
            if worker.is_finished() {
                self.worker = None;
            }
        }
    }

    fn building(&self) -> bool {
        self.worker.is_some()
    }

    /// Draws the whole window into `ui`.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.poll_background();
        if self.building() || self.job.is_some() || self.fetch.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        self.apply_look(ui.ctx());
        self.refresh_problems();
        self.keyboard(ui.ctx());
        let fold = layout::fold_for(ui.max_rect().size());
        egui::Panel::bottom("build")
            .resizable(!fold.dock)
            .default_size(if fold.dock { 40.0 } else { 150.0 })
            .frame(egui::Frame::new().inner_margin(10.0))
            .show(ui, |ui| {
                dock::tab_row(ui, dock::TABS, self);
                if !fold.dock {
                    ui.add_space(4.0);
                    if let Some(tab) = dock::TABS.iter().find(|t| t.id == self.dock) {
                        (tab.show)(self, ui);
                    }
                }
            });
        if fold.left && !self.left_open {
            egui::Panel::left("settings-strip")
                .resizable(false)
                .exact_size(22.0)
                .frame(egui::Frame::new())
                .show(ui, |ui| {
                    if ui
                        .button("›")
                        .on_hover_text("Open the panel of this level")
                        .clicked()
                    {
                        self.left_open = true;
                    }
                });
        } else {
            egui::Panel::left("settings")
                .resizable(false)
                .exact_size(SETTINGS_WIDTH)
                .frame(egui::Frame::new().inner_margin(12.0))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.left_panel(ui));
                });
        }
        if self.has_rail() {
            egui::Panel::left("rail")
                .resizable(false)
                .exact_size(rail::RAIL_WIDTH)
                .frame(egui::Frame::new())
                .show(ui, |ui| rail::rail_ui(self, ui));
        }
        egui::Panel::right("stack")
            .resizable(false)
            .exact_size(RIGHT_WIDTH)
            .frame(egui::Frame::new().inner_margin(12.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        stack::stack_ui(self, ui);
                        self.properties_ui(ui);
                    });
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| self.middle(ui));
        if self.help_open {
            self.help_ui(ui.ctx());
        }
        prefs_window::show(self, ui.ctx());
    }

    /// The interface scale and the animation time of the settings.
    fn apply_look(&self, ctx: &egui::Context) {
        let scale = self.prefs.map.interface_scale;
        if (ctx.zoom_factor() - scale).abs() > f32::EPSILON {
            ctx.set_zoom_factor(scale);
        }
        let time = if self.prefs.map.no_unfold_animation {
            0.0
        } else {
            egui::Style::default().animation_time
        };
        if (ctx.global_style().animation_time - time).abs() > f32::EPSILON {
            ctx.global_style_mut(|s| s.animation_time = time);
        }
    }

    /// Collects the problems of all sources into the list; the cursor stays where it is when it can.
    fn refresh_problems(&mut self) {
        let mut rows: Vec<problems::Problem> = self.build_problems.clone();
        rows.extend(self.merge_problems.iter().cloned());
        if let Some(error) = &self.form.error {
            rows.push(problems::Problem::note(format!("Not saved: {error}")));
        }
        if self.problems.rows != rows {
            self.problems.rows = rows;
            self.problems.cursor = self
                .problems
                .cursor
                .filter(|c| *c < self.problems.rows.len());
        }
    }

    /// Goes to a problem: the map is centred on it, at the zoom it has, when the problem has a position.
    fn go_to_problem(&mut self, index: usize) {
        self.problems.cursor = Some(index);
        let Some(at) = self.problems.rows.get(index).and_then(|p| p.at) else {
            return;
        };
        if !matches!(self.heightmap, HeightmapState::Ready(_)) {
            return;
        }
        let view = self.views.active_mut();
        let span = view
            .focus()
            .map_or_else(|| self.form.project.map_size_m(), |f| f.span_m);
        view.set_focus(viewport::ViewFocus {
            center: at,
            span_m: span,
        });
    }

    /// Runs the action of the key pressed in this frame, if it can be done.
    fn keyboard(&mut self, ctx: &egui::Context) {
        if self.prefs_window.is_some() {
            return; // the window listens to the keys itself
        }
        let typing = shortcuts::typing(ctx);
        let level = self.panel.active;
        if let Some(action) = ctx.input(|i| shortcuts::action_for(i, &self.bindings, level, typing))
            && shortcuts::action_exists(action, self)
        {
            self.run_action(action);
        }
    }

    /// Does what a key (or a click that stands for one) asks for.
    fn run_action(&mut self, action: shortcuts::Action) {
        use shortcuts::Action;
        match action {
            Action::Level(level) => self.activate(level),
            Action::HideLevel => {
                let level = self.panel.active;
                if !self.panel.hidden.remove(&level) {
                    self.panel.hidden.insert(level);
                }
            }
            Action::ShowOthersInFull => {
                self.panel.show_others_in_full = !self.panel.show_others_in_full;
            }
            Action::Tool(tool) => self.tool = tool,
            Action::SearchItems => self.palette.focus_search = true,
            Action::Settings => self.open_settings(),
            Action::Cancel => {
                if self.help_open {
                    self.help_open = false;
                } else if self.tool != shortcuts::ToolId::SELECT {
                    self.tool = shortcuts::ToolId::SELECT;
                } else {
                    self.selection = None;
                }
            }
            Action::FrameSelection => self.frame_selection(),
            Action::FrameMap => self.frame_map(),
            Action::NextProblem | Action::PreviousProblem => {
                let level = self.panel.active;
                let forward = action == Action::NextProblem;
                let list = &mut self.problems;
                let found = if forward {
                    list.next(level)
                } else {
                    list.previous(level)
                };
                if found.is_some()
                    && let Some(index) = self.problems.cursor
                {
                    self.go_to_problem(index);
                }
            }
            Action::Build => self.start_build(),
            Action::Help => self.help_open = !self.help_open,
            Action::SwitchView => self.views.switch(),
            // the other actions do not exist yet, see `shortcuts::action_exists`
            _ => {}
        }
    }

    /// Makes a level the one you work on. A hidden level is shown again.
    fn activate(&mut self, level: levels::Level) {
        if self.panel.active != level {
            self.tool = shortcuts::ToolId::SELECT;
            self.item = None;
        }
        self.panel.active = level;
        self.panel.hidden.remove(&level);
    }

    fn open_settings(&mut self) {
        if self.prefs_window.is_none() {
            self.prefs_window = Some(prefs_window::PrefsWindow::default());
        }
    }

    fn start_build(&mut self) {
        if self.building() || self.form.error.is_some() {
            return;
        }
        self.log.clear();
        self.build_problems.clear();
        self.log.push(Line::Info("Starting the build".to_string()));
        self.worker = Some(BuildWorker::start(self.form.dir(), (self.locate_tools)()));
    }

    /// The sheet of the shortcuts that work in the active level.
    fn help_ui(&mut self, ctx: &egui::Context) {
        let level = self.panel.active;
        let rows = shortcuts::sheet(self, level);
        let mut open = true;
        egui::Window::new("Keyboard shortcuts")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .shadow(egui::Shadow::NONE)
                    .stroke(egui::Stroke::new(
                        1.5,
                        ctx.global_style().visuals.text_color(),
                    )),
            )
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("shortcut-sheet")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for (label, key) in rows {
                            ui.label(label);
                            ui.label(key);
                            ui.end_row();
                        }
                    });
            });
        self.help_open = open;
    }

    /// The panel of the active level.
    fn left_panel(&mut self, ui: &mut egui::Ui) {
        let Some(def) = self.levels.iter().find(|d| d.level == self.panel.active) else {
            return;
        };
        let items = (def.items)(self);
        if items.is_empty() {
            (def.panel)(self, ui);
        } else {
            palette::palette_ui(self, ui, &items);
        }
    }

    /// Whether the active level has this tool: Select when it can pick, or one of its own.
    fn has_tool(&self, tool: shortcuts::ToolId) -> bool {
        self.rail_tools().iter().any(|(t, _)| *t == tool)
    }

    /// Whether the active level shows the items panel.
    fn has_palette(&self) -> bool {
        self.levels
            .iter()
            .find(|d| d.level == self.panel.active)
            .is_some_and(|d| !(d.items)(self).is_empty())
    }

    /// The band, the view and the status line.
    fn middle(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let band =
            egui::Rect::from_min_size(full.min, egui::vec2(full.width().max(0.0), BAND_HEIGHT));
        ui.painter()
            .rect_filled(band, 0.0, self.panel.band_colour());
        let status_h = STATUS_HEIGHT.min((full.height() - BAND_HEIGHT).max(0.0));
        let status =
            egui::Rect::from_min_max(egui::pos2(full.min.x, full.max.y - status_h), full.max);
        let map = egui::Rect::from_min_max(
            egui::pos2(full.min.x, band.max.y),
            egui::pos2(full.max.x, status.min.y.max(band.max.y)),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(map), |ui| self.map_ui(ui));
        let text = self.status_text();
        ui.scope_builder(egui::UiBuilder::new().max_rect(status), |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(text);
            });
        });
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let p = &mut self.form.project;
        ui.heading("Project");
        ui.add_space(6.0);
        egui::Grid::new("settings-grid")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                changed |= field(ui, "Name", |ui| ui.text_edit_singleline(&mut p.name));
                changed |= field(ui, "Latitude", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut p.area.center_lat)
                            .speed(0.001)
                            .suffix("°"),
                    )
                });
                changed |= field(ui, "Longitude", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut p.area.center_lon)
                            .speed(0.001)
                            .suffix("°"),
                    )
                });
                changed |= field(ui, "Real extent", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut p.area.real_extent_m)
                            .speed(50.0)
                            .suffix(" m"),
                    )
                });
                changed |= field(ui, "Horizontal scale", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut p.scale.horizontal)
                            .speed(0.01)
                            .range(0.0..=1.0),
                    )
                });
                changed |= field(ui, "Height scale", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut p.scale.height)
                            .speed(0.01)
                            .range(0.0..=2.0),
                    )
                });
                changed |= field(ui, "Imagery brightness", |ui| {
                    ui.add(
                        egui::DragValue::new(&mut p.imagery.brightness)
                            .speed(0.01)
                            .range(0.25..=4.0),
                    )
                });
                let terrain = field(ui, "Terrain grid", |ui| {
                    power_of_two(ui, "terrain", &mut p.grid.terrain_size, 256, 8192)
                });
                if terrain {
                    // DayZ needs 8 terrain cells per land cell, so the land grid is not a choice.
                    p.grid.land_size = p.grid.terrain_size / LAND_RATIO;
                }
                changed |= terrain;
                field(ui, "Material grid", |ui| {
                    ui.label(format!("{0} × {0} land cells", p.grid.land_size))
                });
            });
        ui.add_space(8.0);
        let map = p.map_size_m();
        ui.label(format!(
            "Map: {map:.0} m, vertex every {:.2} m",
            p.terrain_cell_m()
        ));
        ui.add_space(8.0);
        elevation_and_ground_files_ui(ui, p);
        if changed {
            self.settings_changed();
        }
        ui.add_space(8.0);
        self.imagery_section(ui);
        if let Some(error) = &self.form.error {
            ui.add_space(8.0);
            ui.colored_label(ERROR, format!("Not saved: {error}"));
        }
    }

    fn build_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let can_build = !self.building() && self.form.error.is_none();
            let button = egui::Button::new(RichText::new("Build").color(Color32::WHITE).strong())
                .fill(GREEN);
            if ui.add_enabled(can_build, button).clicked() {
                self.start_build();
            }
            if self.building() {
                ui.spinner();
            }
        });
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in &self.log {
                    match line {
                        Line::Info(text) => ui.label(RichText::new(text).monospace()),
                        Line::Warning(text) => ui.colored_label(
                            ui.style().visuals.warn_fg_color,
                            RichText::new(format!("Warning: {text}")).monospace(),
                        ),
                        Line::Error(text) => ui.colored_label(
                            ERROR,
                            RichText::new(format!("Failed: {text}")).monospace(),
                        ),
                        Line::Result(text) => {
                            ui.colored_label(GREEN, RichText::new(text).monospace())
                        }
                    };
                }
            });
    }

    fn map_ui(&mut self, ui: &mut egui::Ui) {
        let data = match &self.heightmap {
            HeightmapState::Ready(data) => Arc::clone(data),
            HeightmapState::NoDem => {
                ui.centered_and_justified(|ui| {
                    ui.label("Add elevation files to see the map.");
                });
                return;
            }
            HeightmapState::Loading => {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                });
                return;
            }
            HeightmapState::Failed(error) => {
                ui.colored_label(ERROR, format!("The map cannot be drawn: {error}"));
                return;
            }
        };
        if self.panel.base == Overlay::Imagery && data.imagery.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    "No imagery yet. List GeoTIFF files in sources.imagery in naves.toml, then reopen the project.",
                );
            });
            return;
        }
        let rect = ui.available_rect_before_wrap();
        let layers = layers::visible(
            &self.panel,
            &self.levels,
            f32::from(self.prefs.map.fade_percent) / 100.0,
        );
        let frame = ViewFrame {
            data: &data,
            project: &self.form.project,
            base: self.panel.base,
            layers: &layers,
            invert_zoom: self.prefs.map.invert_zoom,
            generation: self.generation,
        };
        let events = self.views.active_mut().show(ui, rect, &frame);
        self.draw_markers(ui, rect);
        self.draw_selection(ui, rect);
        for event in &events {
            self.route(event);
        }
        #[cfg(test)]
        {
            self.seen_events.extend(events);
            self.seen_rect = rect;
        }
    }

    /// An event of the view goes to the tool in use; Select picks.
    fn route(&mut self, event: &ViewEvent) {
        if self.tool == shortcuts::ToolId::SELECT {
            if let ViewEvent::Click { at, modifiers } = *event {
                self.select(at, modifiers);
            }
            return;
        }
        let tool = self
            .levels
            .iter()
            .find(|d| d.level == self.panel.active)
            .and_then(|d| d.tools.iter().find(|t| t.tool == self.tool))
            .and_then(|t| t.on_event);
        if let Some(on_event) = tool {
            on_event(self, event);
        }
    }

    /// What the status line says: the tool in use, or the level.
    fn status_text(&self) -> &'static str {
        let Some(def) = self.levels.iter().find(|d| d.level == self.panel.active) else {
            return "";
        };
        def.tools
            .iter()
            .find(|t| t.tool == self.tool)
            .map_or(def.status, |t| t.status)
    }

    /// A click: picks in the active level, or with Alt in the highest level that has something there.
    fn select(&mut self, at: viewport::WorldPoint, modifiers: egui::Modifiers) {
        self.selection = selection::pick_at(self, at, modifiers);
        if modifiers.alt
            && let Some(selection) = &self.selection
        {
            self.activate(selection.level);
        }
    }

    /// A diamond with a white edge for each shown problem that has a position.
    fn draw_markers(&self, ui: &egui::Ui, rect: egui::Rect) {
        if !self.problems.markers {
            return;
        }
        let painter = ui.painter().with_clip_rect(rect);
        for index in self.problems.shown(self.panel.active) {
            let Some(at) = self.problems.rows.get(index).and_then(|p| p.at) else {
                continue;
            };
            if let Some(pos) = self.views.active().project(at, rect) {
                let diamond = |r: f32| {
                    vec![
                        pos + egui::vec2(0.0, -r),
                        pos + egui::vec2(r, 0.0),
                        pos + egui::vec2(0.0, r),
                        pos + egui::vec2(-r, 0.0),
                    ]
                };
                painter.add(egui::Shape::convex_polygon(
                    diamond(9.0),
                    ERROR,
                    egui::Stroke::new(2.5, egui::Color32::WHITE),
                ));
            }
        }
    }

    /// The marker of the selection, where the active view says the point is.
    fn draw_selection(&self, ui: &egui::Ui, rect: egui::Rect) {
        let Some(selection) = &self.selection else {
            return;
        };
        if let Some(pos) = self.views.active().project(selection.at, rect) {
            let painter = ui.painter().with_clip_rect(rect);
            painter.circle_stroke(pos, 9.0, egui::Stroke::new(5.0, egui::Color32::WHITE));
            painter.circle_stroke(pos, 9.0, egui::Stroke::new(2.5, egui::Color32::BLACK));
        }
    }

    /// Frames the selection in the active view, keeping the zoom unless the selection has a size.
    fn frame_selection(&mut self) {
        if !matches!(self.heightmap, HeightmapState::Ready(_)) {
            return;
        }
        let Some(selection) = &self.selection else {
            return;
        };
        let view = self.views.active_mut();
        let span = selection
            .span_m
            .or_else(|| view.focus().map(|f| f.span_m))
            .unwrap_or(FRAME_SPAN_M);
        view.set_focus(viewport::ViewFocus {
            center: selection.at,
            span_m: span,
        });
    }

    /// Shows the whole map in the active view.
    fn frame_map(&mut self) {
        if !matches!(self.heightmap, HeightmapState::Ready(_)) {
            return;
        }
        let size = self.form.project.map_size_m();
        self.views.active_mut().set_focus(viewport::ViewFocus {
            center: viewport::WorldPoint {
                x: size / 2.0,
                y: size / 2.0,
            },
            span_m: size,
        });
    }

    /// What the selection is, under the stack.
    fn properties_ui(&self, ui: &mut egui::Ui) {
        let Some(selection) = &self.selection else {
            return;
        };
        ui.add_space(14.0);
        ui.label(RichText::new(&selection.title).strong());
        egui::Grid::new("properties")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                for (name, value) in &selection.rows {
                    ui.label(name);
                    ui.label(value);
                    ui.end_row();
                }
            });
        if let Some(origin) = &selection.origin {
            ui.label(origin);
        }
    }

    /// Adds a level that only tests build.
    #[cfg(test)]
    fn with_extra_level(mut self, def: &'static LevelDef) -> Self {
        self.levels.push(def);
        self.levels.sort_by_key(|d| d.level.digit());
        self
    }

    #[cfg(test)]
    fn map_texture_is_empty(&self) -> bool {
        !self.views.active().holds_picture()
    }

    #[cfg(test)]
    fn with_views(mut self, views: Vec<Box<dyn viewport::Viewport>>) -> Self {
        self.views = ViewportSet::new(views);
        self
    }
}

/// The elevation, land cover and OpenStreetMap files of the project, with what to do when there are none.
fn elevation_and_ground_files_ui(ui: &mut egui::Ui, p: &Project) {
    ui.label(RichText::new("Elevation files").strong());
    dem_files_ui(ui, p);
    ui.add_space(8.0);
    ground_files_ui(ui, p);
}

/// The elevation files, or what to do when there are none.
fn dem_files_ui(ui: &mut egui::Ui, p: &Project) {
    if p.sources.dem.is_empty() {
        ui.label("None yet. Add DEM files to sources.dem in naves.toml, then reopen the project.");
    }
    for file in &p.sources.dem {
        ui.label(file.display().to_string());
    }
}

/// The land cover and OpenStreetMap files, and what the ground follows when there are none.
fn ground_files_ui(ui: &mut egui::Ui, p: &Project) {
    source_list(ui, "Land cover files", &p.sources.land_cover);
    ui.add_space(8.0);
    source_list(ui, "OpenStreetMap files", &p.sources.osm);
    if p.sources.land_cover.is_empty() && p.sources.osm.is_empty() {
        ui.add_space(4.0);
        ui.label(
            "Without them the ground follows slope and altitude. List files in sources.land_cover and \
             sources.osm in naves.toml, then reopen the project.",
        );
    }
}

/// A heading and the files of one source, or "None." when there are none.
fn source_list(ui: &mut egui::Ui, title: &str, files: &[PathBuf]) {
    ui.label(RichText::new(title).strong());
    if files.is_empty() {
        ui.label("None.");
    }
    for file in files {
        ui.label(file.display().to_string());
    }
}

/// One labelled row of the settings grid; the label is also the widget's accessible name.
/// Returns whether the widget changed the value.
fn field(
    ui: &mut egui::Ui,
    label: &str,
    widget: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> bool {
    let name = ui.label(label);
    let response = widget(ui).labelled_by(name.id);
    ui.end_row();
    response.changed()
}

/// The project values the map depends on: the area, the scales, the grid and the files of every source.
type HeightmapInputs = (
    Area,
    Scale,
    Grid,
    Vec<PathBuf>,
    Vec<PathBuf>,
    Vec<PathBuf>,
    Vec<ImageryFile>,
);

fn heightmap_inputs(project: &Project) -> HeightmapInputs {
    (
        project.area.clone(),
        project.scale.clone(),
        project.grid.clone(),
        project.sources.dem.clone(),
        project.sources.land_cover.clone(),
        project.sources.osm.clone(),
        project.sources.imagery.clone(),
    )
}

/// A drop-down with the powers of two from `min` to `max`; the response is marked changed when the value is.
fn power_of_two(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut u32,
    min: u32,
    max: u32,
) -> egui::Response {
    let before = *value;
    let mut response = egui::ComboBox::from_id_salt(id)
        .selected_text(value.to_string())
        .show_ui(ui, |ui| {
            let mut v = min;
            while v <= max {
                ui.selectable_value(value, v, v.to_string());
                v *= 2;
            }
        })
        .response;
    if *value != before {
        response.mark_changed();
    }
    response
}

/// Opens the window for the project in `dir`.
///
/// # Errors
///
/// The project cannot be loaded or the window cannot be created.
pub fn run(dir: &Path) -> anyhow::Result<()> {
    let mut app = NavesApp::open(
        dir,
        Box::new(|| crate::user_prefs::locate_tools().map_err(|e| e.to_string())),
    )?;
    if let Some(path) = crate::user_prefs::path() {
        app = app.with_preferences(path);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 760.0])
            .with_title(app.title()),
        ..Default::default()
    };
    eframe::run_native(
        "naves",
        options,
        Box::new(move |cc| {
            theme::apply_theme(&cc.egui_ctx);
            Ok(Box::new(Shell(app)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("cannot open the window: {e}"))
}

struct Shell(NavesApp);

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.show(ui);
    }
}

#[cfg(test)]
mod tests;

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The graphical editor: project settings, a top-down map and the build.
//!
//! The widget code is thin; the logic lives in plain structs (`settings`, `map_view`, `worker`).
//! Look: the manual's palette. A lichen-green ground, ink text, village-green accents, and the map as the
//! one loud thing on the screen.

mod imagery;
mod map_view;
mod settings;
mod worker;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use naves_core::imagery::TileImagery;
use naves_core::imagery::catalog::ImageryOption;
use naves_core::project::{Area, Grid, ImageryFile, LAND_RATIO, Project, Scale};
use naves_core::render::{Overlay, RenderRequest, render_topdown};
use naves_core::surface::SurfaceClass;
use naves_export::build::BuildOutput;
use naves_export::imagery_fetch::Http;
use naves_export::tools::Tools;

use imagery::{FetchJob, OwnFile};
use map_view::MapView;
use settings::SettingsForm;
use worker::{BuildMsg, BuildWorker, HeightmapJob, HeightmapState, MapData};

const GREEN: Color32 = Color32::from_rgb(0x2e, 0x6a, 0x4c);
const BROWN: Color32 = Color32::from_rgb(0x8c, 0x6a, 0x45);
const ERROR: Color32 = Color32::from_rgb(0xb0, 0x3a, 0x2e);
const INK: Color32 = Color32::from_rgb(0x1d, 0x29, 0x24);
const LICHEN: Color32 = Color32::from_rgb(0xf2, 0xf5, 0xec);
const PANEL: Color32 = Color32::from_rgb(0xe7, 0xed, 0xe0);
/// Widest picture the map view renders; larger windows scale it up.
const MAX_RENDER_WIDTH: f32 = 1024.0;
const SETTINGS_WIDTH: f32 = 300.0;

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
    view: Option<MapView>,
    overlay: Overlay,
    heightmap: HeightmapState,
    job: Option<HeightmapJob>,
    /// The settings changed while `job` was running, so its result is out of date.
    reload_wanted: bool,
    /// The project values the current heightmap depends on.
    heightmap_inputs: HeightmapInputs,
    texture: Option<(RenderRequest, egui::TextureHandle)>,
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

fn to_px(v: f32) -> u32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=8192 first"
    )]
    let px = v.round().clamp(1.0, 8192.0) as u32;
    px
}

/// The look of the editor: the manual's palette on egui's light theme.
pub fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = LICHEN;
    visuals.window_fill = LICHEN;
    visuals.extreme_bg_color = Color32::WHITE;
    visuals.override_text_color = Some(INK);
    visuals.selection.bg_fill = GREEN;
    visuals.selection.stroke.color = Color32::WHITE;
    visuals.hyperlink_color = GREEN;
    visuals.widgets.hovered.bg_stroke.color = GREEN;
    visuals.widgets.active.bg_fill = GREEN;
    ctx.set_visuals(visuals);
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
            view: None,
            overlay: Overlay::Hillshade,
            heightmap: HeightmapState::NoDem,
            job: None,
            reload_wanted: false,
            heightmap_inputs: inputs,
            texture: None,
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

    /// The window title.
    pub fn title(&self) -> String {
        format!("Náves – {}", self.form.project.name)
    }

    fn reload_heightmap(&mut self) {
        self.texture = None;
        self.view = None;
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
            self.texture = None;
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
                        if let Some(imagery) = &data.imagery {
                            self.log
                                .extend(imagery.warnings.iter().cloned().map(Line::Warning));
                        }
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
                    BuildMsg::Failed(error) => self.log.push(Line::Error(error)),
                    BuildMsg::Done(BuildOutput {
                        mod_dir,
                        mission_dir,
                        warnings,
                    }) => {
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
        egui::Panel::left("settings")
            .resizable(false)
            .exact_size(SETTINGS_WIDTH)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(12.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.settings_ui(ui));
            });
        egui::Panel::bottom("build")
            .resizable(true)
            .default_size(150.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(10.0))
            .show(ui, |ui| self.build_ui(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(LICHEN))
            .show(ui, |ui| self.map_ui(ui));
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
        ui.label(
            RichText::new(format!(
                "Map: {map:.0} m, vertex every {:.2} m",
                p.terrain_cell_m()
            ))
            .color(BROWN),
        );
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
                self.log.clear();
                self.log.push(Line::Info("Starting the build".to_string()));
                self.worker = Some(BuildWorker::start(self.form.dir(), (self.locate_tools)()));
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
                            BROWN,
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
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.overlay, Overlay::Hillshade, "Hillshade");
            ui.selectable_value(&mut self.overlay, Overlay::Slope, "Slope");
            ui.selectable_value(&mut self.overlay, Overlay::Height, "Height");
            ui.selectable_value(&mut self.overlay, Overlay::Surface, "Surface");
            ui.selectable_value(&mut self.overlay, Overlay::Imagery, "Imagery");
            if self.overlay == Overlay::Surface {
                ui.add_space(12.0);
                surface_legend(ui);
            }
            if self.overlay == Overlay::Imagery {
                ui.add_space(12.0);
                ui.label("Ground without imagery is checkered.");
            }
        });
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
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let map_size = self.form.project.map_size_m();
        let view = self
            .view
            .get_or_insert_with(|| MapView::fit(map_size, w, h));
        if response.dragged() {
            let d = response.drag_delta();
            view.pan_px(f64::from(d.x), f64::from(d.y));
        }
        if let Some(pos) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let factor = (-f64::from(scroll) * 0.002).exp();
                view.zoom_at(
                    f64::from(pos.x - rect.left()),
                    f64::from(pos.y - rect.top()),
                    w,
                    h,
                    factor,
                );
            }
        }
        let view = *view;
        self.draw_map(ui, &data, view, rect);
    }

    fn draw_map(&mut self, ui: &mut egui::Ui, data: &MapData, view: MapView, rect: egui::Rect) {
        let (x0, y0, x1, y1) = view.bounds(f64::from(rect.width()), f64::from(rect.height()));
        if self.overlay == Overlay::Imagery && data.imagery.is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    "No imagery yet. List GeoTIFF files in sources.imagery in naves.toml, then reopen the project.",
                );
            });
            return;
        }
        let request = RenderRequest {
            x0,
            y0,
            x1,
            y1,
            px: to_px(rect.width().min(MAX_RENDER_WIDTH)),
            overlay: self.overlay,
        };
        if self.texture.as_ref().is_none_or(|(r, _)| *r != request)
            && let Ok(image) = render_topdown(
                &data.heightmap,
                &request,
                Some(&data.surfaces),
                data.imagery
                    .as_ref()
                    .map(|loaded| TileImagery {
                        composite: &loaded.composite,
                        project: &self.form.project,
                    })
                    .as_ref(),
            )
        {
            let size = [image.width() as usize, image.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
            let handle = ui
                .ctx()
                .load_texture("map", color, egui::TextureOptions::LINEAR);
            self.texture = Some((request, handle));
        }
        if let Some((_, texture)) = &self.texture {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter()
                .with_clip_rect(rect)
                .image(texture.id(), rect, uv, Color32::WHITE);
        }
    }
}

/// The elevation, land cover and OpenStreetMap files of the project, with what to do when there are none.
fn elevation_and_ground_files_ui(ui: &mut egui::Ui, p: &Project) {
    ui.label(RichText::new("Elevation files").strong());
    if p.sources.dem.is_empty() {
        ui.label("None yet. Add DEM files to sources.dem in naves.toml, then reopen the project.");
    }
    for file in &p.sources.dem {
        ui.label(file.display().to_string());
    }
    ui.add_space(8.0);
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

/// The ground classes with the colour they have in the surface overlay.
fn surface_legend(ui: &mut egui::Ui) {
    for class in SurfaceClass::ALL {
        let [r, g, b] = class.rgb();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 2.0, Color32::from_rgb(r, g, b));
        ui.label(class.name());
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
    let app = NavesApp::open(dir, Box::new(|| Tools::locate().map_err(|e| e.to_string())))?;
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
            apply_theme(&cc.egui_ctx);
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

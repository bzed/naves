// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The settings window: shortcuts, the map and the external tools. It belongs to the person, not to the map, so it
//! applies to every project. The code says `Preferences` because `settings.rs` is the project's form.

use std::path::PathBuf;

use eframe::egui::{self, Event, Key, Modifiers, RichText, WidgetInfo};
use naves_export::tools::{Origin, ToolOrigin};

use super::NavesApp;
use super::levels::Level;
use super::shortcuts::{self, Action, Binding, SHORTCUTS, Scope, Shortcut};
use super::stack::{BASE_MAPS, overlay_id};
use super::theme::{ERROR, GROUND, INK};
use naves_core::render::MapLayer;

/// The tabs, in the order of the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefsTab {
    /// Every shortcut.
    Keyboard,
    /// The wheel, the fade, the scale and what a project opens with.
    Map,
    /// The paths of armake2, Steam, DayZ Tools, DayZ, DayZ Server and Proton.
    ExternalTools,
}

impl PrefsTab {
    const ALL: [(PrefsTab, &'static str); 3] = [
        (PrefsTab::Keyboard, "Keyboard"),
        (PrefsTab::Map, "Map"),
        (PrefsTab::ExternalTools, "External tools"),
    ];
}

/// A key that is already in use, waiting for Replace or Cancel.
#[derive(Debug, Clone, Copy)]
pub struct Pending {
    /// The key that was pressed.
    pub binding: Binding,
    /// The id of the shortcut that wants it.
    pub id: &'static str,
    /// The shortcut that has it.
    pub other: &'static Shortcut,
}

/// The state of the window.
#[derive(Debug)]
pub struct PrefsWindow {
    /// The tab in front.
    pub tab: PrefsTab,
    /// The shortcut that listens for its next key.
    pub capturing: Option<&'static str>,
    /// A key that is already in use.
    pub pending: Option<Pending>,
    /// What is typed in the search field.
    pub search: String,
    /// Why the last key was not taken.
    message: Option<String>,
    /// Where each tool comes from; looked up when the tab opens and when a path changes.
    origins: Option<Vec<ToolOrigin>>,
    /// The text of the path fields.
    paths: Vec<String>,
}

impl Default for PrefsWindow {
    fn default() -> Self {
        Self {
            tab: PrefsTab::Keyboard,
            capturing: None,
            pending: None,
            search: String::new(),
            message: None,
            origins: None,
            paths: Vec::new(),
        }
    }
}

/// Esc, Enter and F1 always work; Tab and Shift+Tab always move through the panels.
fn why_not(binding: Binding) -> Option<&'static str> {
    let plain = binding.modifiers == Modifiers::NONE;
    match binding.key {
        Key::Tab
            if binding.modifiers == Modifiers::NONE || binding.modifiers == Modifiers::SHIFT =>
        {
            Some(
                "Tab and Shift+Tab cannot be assigned, so that the way through the panels always works.",
            )
        }
        Key::Enter if plain => Some("Enter cannot be assigned: it finishes a line or an area."),
        Key::F1 if plain => Some("F1 cannot be assigned: it is the way to the help."),
        _ => None,
    }
}

/// The actions whose keys cannot be changed.
fn fixed(action: Action) -> bool {
    matches!(action, Action::Cancel | Action::Finish | Action::Help)
}

/// Draws the window if it is open and listens to its keys.
pub fn show(app: &mut NavesApp, ctx: &egui::Context) {
    let Some(mut window) = app.prefs_window.take() else {
        return;
    };
    let mut close = false;
    let escape = ctx.input(|i| i.key_pressed(Key::Escape));
    let pressed = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                repeat: false,
                ..
            } => Some(Binding {
                key: *key,
                modifiers: *modifiers,
            }),
            _ => None,
        })
    });
    // Esc cancels what is going on first and closes the window last
    if escape {
        if window.capturing.is_some() || window.pending.is_some() {
            window.capturing = None;
            window.pending = None;
            window.message = None;
        } else {
            close = true;
        }
    } else if let (Some(id), Some(binding)) = (window.capturing, pressed) {
        capture(app, &mut window, id, binding);
    }
    let modal = egui::Modal::new(egui::Id::unique("settings")).frame(
        egui::Frame::window(&ctx.global_style())
            .fill(GROUND)
            .shadow(egui::Shadow::NONE)
            .stroke(egui::Stroke::new(1.5, INK))
            .inner_margin(16.0),
    );
    let response = modal.show(ctx, |ui| {
        ui.set_width(640.0);
        ui.set_min_height(420.0);
        ui.heading("Settings");
        ui.label("These apply to every project. Each project's own values are in level 0.");
        problems_ui(app, ui);
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(130.0);
                for (tab, name) in PrefsTab::ALL {
                    let selected = window.tab == tab;
                    let text = if selected {
                        RichText::new(name).strong()
                    } else {
                        RichText::new(name)
                    };
                    let r = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                    if selected {
                        let bar = egui::Rect::from_min_size(
                            egui::pos2(r.rect.left() - 8.0, r.rect.top()),
                            egui::vec2(3.0, r.rect.height()),
                        );
                        ui.painter().rect_filled(bar, 0.0, super::theme::GREEN);
                    }
                    if r.has_focus() {
                        ui.painter().rect_stroke(
                            r.rect.expand(2.0),
                            2.0,
                            egui::Stroke::new(2.0, super::theme::GREEN),
                            egui::StrokeKind::Outside,
                        );
                    }
                    r.widget_info(|| {
                        WidgetInfo::selected(egui::Role::Tab, true, selected, name.to_string())
                    });
                    if r.clicked() {
                        window.tab = tab;
                        window.origins = None;
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| match window.tab {
                PrefsTab::Keyboard => keyboard_tab(app, &mut window, ui),
                PrefsTab::Map => map_tab(app, ui),
                PrefsTab::ExternalTools => tools_tab(app, &mut window, ui),
            });
        });
    });
    // Esc is handled above; a click on the dimmed ground closes it too, unless a key is being chosen
    let outside = response.backdrop_response.clicked()
        && window.capturing.is_none()
        && window.pending.is_none();
    if close || outside {
        return;
    }
    app.prefs_window = Some(window);
}

/// What is wrong with the file, and the last failed save.
fn problems_ui(app: &NavesApp, ui: &mut egui::Ui) {
    if let Some(error) = &app.prefs_error {
        ui.colored_label(
            ERROR,
            format!("The settings file is damaged: {error}. It is left as it is. Changes hold until you close Náves and are not saved."),
        );
    }
    if let Some(error) = &app.prefs_save_error {
        ui.colored_label(ERROR, error);
    }
}

/// A key was pressed while a row listens.
fn capture(app: &mut NavesApp, window: &mut PrefsWindow, id: &'static str, binding: Binding) {
    if let Some(reason) = why_not(binding) {
        window.message = Some(reason.to_string());
        return; // it keeps listening
    }
    window.message = None;
    if let Some(other) = app.bindings.conflict(binding, id) {
        window.pending = Some(Pending { binding, id, other });
    } else {
        app.bindings.set(id, binding);
        app.save_shortcuts();
    }
    window.capturing = None;
}

/// "R is Scale in every level." or "S is Smooth in Terrain."
fn conflict_text(binding: Binding, other: &Shortcut) -> String {
    match other.scope {
        Scope::Everywhere => format!("{binding} is {} in every level.", other.label),
        Scope::Level(level) => format!("{binding} is {} in {}.", other.label, level.name()),
    }
}

fn keyboard_tab(app: &mut NavesApp, window: &mut PrefsWindow, ui: &mut egui::Ui) {
    for note in app
        .binding_notes
        .iter()
        .chain(app.bindings.duplicates().iter())
    {
        ui.colored_label(ERROR, note);
    }
    if let Some(message) = &window.message {
        ui.colored_label(ERROR, message);
    }
    if let Some(pending) = window.pending {
        ui.horizontal(|ui| {
            ui.label(conflict_text(pending.binding, pending.other));
            if ui.button("Replace").clicked() {
                app.bindings.clear(pending.other.id);
                app.bindings.set(pending.id, pending.binding);
                app.save_shortcuts();
                window.pending = None;
            }
            if ui.button("Cancel").clicked() {
                window.pending = None;
            }
        });
    }
    ui.horizontal(|ui| {
        ui.label("Search shortcuts");
        ui.text_edit_singleline(&mut window.search);
    });
    ui.add_space(6.0);
    let needle = window.search.trim().to_lowercase();
    let mut groups: Vec<(String, Vec<&'static Shortcut>)> = Vec::new();
    let listed = |s: &&Shortcut| {
        shortcuts::action_built(s.action, app)
            && (needle.is_empty() || s.label.to_lowercase().contains(&needle))
    };
    groups.push((
        "Everywhere".to_string(),
        SHORTCUTS
            .iter()
            .filter(|s| s.scope == Scope::Everywhere)
            .filter(listed)
            .collect(),
    ));
    for def in &app.levels {
        let rows: Vec<&Shortcut> = SHORTCUTS
            .iter()
            .filter(|s| s.scope == Scope::Level(def.level))
            .filter(listed)
            .collect();
        groups.push((def.level.name().to_string(), rows));
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(330.0)
        .show(ui, |ui| {
            for (name, rows) in groups.into_iter().filter(|(_, r)| !r.is_empty()) {
                ui.label(RichText::new(&name).strong());
                egui::Grid::new(("rows", name.as_str()))
                    .num_columns(3)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for s in rows {
                            row(app, window, ui, s);
                        }
                    });
                ui.add_space(8.0);
            }
        });
    ui.add_space(6.0);
    if ui.button("Reset all to defaults").clicked() {
        app.bindings.reset_all();
        app.save_shortcuts();
    }
}

fn row(app: &mut NavesApp, window: &mut PrefsWindow, ui: &mut egui::Ui, s: &'static Shortcut) {
    ui.label(s.label);
    let key = app
        .bindings
        .key_of(s.id)
        .map_or_else(|| "no key".to_string(), |b| b.to_string());
    let listening = window.capturing == Some(s.id);
    let text = if listening { "Press a key" } else { &key };
    let locked = fixed(s.action);
    let response = ui.add_enabled(
        !locked,
        egui::Button::new(text).min_size(egui::vec2(110.0, 0.0)),
    );
    response.widget_info(|| {
        WidgetInfo::labeled(
            egui::Role::Button,
            !locked,
            format!("{} shortcut, {text}", s.label),
        )
    });
    if locked {
        response
            .clone()
            .on_disabled_hover_text("This key cannot be changed.");
    }
    if response.clicked() {
        window.capturing = Some(s.id);
        window.pending = None;
        window.message = None;
    }
    if app.bindings.is_changed(s) {
        ui.horizontal(|ui| {
            ui.label("changed");
            if ui.button("Reset").clicked() {
                app.bindings.reset(s.id);
                app.save_shortcuts();
            }
        });
    } else {
        ui.label("");
    }
    ui.end_row();
}

fn map_tab(app: &mut NavesApp, ui: &mut egui::Ui) {
    let mut changed = false;
    let map = &mut app.prefs.map;
    changed |= ui
        .checkbox(
            &mut map.invert_zoom,
            "Invert the zoom direction of the wheel",
        )
        .changed();
    ui.horizontal(|ui| {
        ui.label("Other levels are drawn at");
        changed |= ui
            .add(
                egui::DragValue::new(&mut map.fade_percent)
                    .range(0..=100)
                    .suffix(" %"),
            )
            .changed();
    });
    ui.horizontal(|ui| {
        ui.label("Interface scale");
        changed |= ui
            .add(
                egui::DragValue::new(&mut map.interface_scale)
                    .range(0.5..=3.0)
                    .speed(0.05)
                    .fixed_decimals(2),
            )
            .changed();
    });
    changed |= ui
        .checkbox(&mut map.no_unfold_animation, "Unfold without animation")
        .changed();
    ui.add_space(8.0);
    ui.label(RichText::new("A project opens with").strong());
    ui.horizontal(|ui| {
        ui.label("Base map");
        egui::ComboBox::from_id_salt("start-base")
            .selected_text(
                BASE_MAPS
                    .iter()
                    .find(|(o, _)| overlay_id(*o) == map.start_base)
                    .map_or("Relief", |(_, name)| *name),
            )
            .show_ui(ui, |ui| {
                for (overlay, name) in BASE_MAPS {
                    changed |= ui
                        .selectable_value(
                            &mut map.start_base,
                            overlay_id(overlay).to_string(),
                            name,
                        )
                        .changed();
                }
            });
    });
    let names: Vec<(MapLayer, &str, Level)> = app
        .levels
        .iter()
        .flat_map(|d| d.layers.iter().map(move |l| (l.layer, l.name, d.level)))
        .collect();
    for (layer, name, _) in names {
        let mut on = map.start_layers.iter().any(|id| id == layer.id());
        if ui.checkbox(&mut on, name).changed() {
            map.start_layers.retain(|id| id != layer.id());
            if on {
                map.start_layers.push(layer.id().to_string());
            }
            changed = true;
        }
    }
    if changed {
        app.save_prefs();
    }
}

fn origin_words(from: Origin, var: &str) -> String {
    match from {
        Origin::Environment => "From an environment variable".to_string(),
        Origin::Settings => "From the settings".to_string(),
        Origin::Found => "Found by searching".to_string(),
        Origin::Missing => {
            let _ = var;
            "Not found".to_string()
        }
    }
}

fn tools_tab(app: &mut NavesApp, window: &mut PrefsWindow, ui: &mut egui::Ui) {
    ui.label("An environment variable wins over these paths, and these over the search.");
    ui.add_space(6.0);
    let origins = window
        .origins
        .get_or_insert_with(|| (app.origins)(&app.prefs.tools))
        .clone();
    if window.paths.len() != origins.len() {
        window.paths = origins
            .iter()
            .map(|o| current_setting(app, o.tool))
            .collect();
    }
    let mut changed = None;
    egui::Grid::new("tools")
        .num_columns(2)
        .spacing([16.0, 12.0])
        .show(ui, |ui| {
            for (i, origin) in origins.iter().enumerate() {
                ui.label(RichText::new(origin.tool).strong());
                ui.vertical(|ui| {
                    ui.label(
                        origin
                            .path
                            .as_ref()
                            .map_or_else(|| "none".to_string(), |p| p.display().to_string()),
                    );
                    ui.label(RichText::new(origin_words(origin.from, "")).weak());
                    if let Some(text) = window.paths.get_mut(i) {
                        let r = ui.add(
                            egui::TextEdit::singleline(text)
                                .hint_text("Path in the settings")
                                .desired_width(340.0),
                        );
                        if r.changed() {
                            changed = Some((origin.tool, text.clone()));
                        }
                    }
                });
                ui.end_row();
            }
        });
    if let Some((tool, text)) = changed {
        set_tool_path(app, tool, &text);
        window.origins = None;
        app.save_prefs();
    }
    if let Some(error) = &app.prefs_save_error {
        ui.colored_label(ERROR, error);
    }
}

fn tool_slot<'a>(app: &'a mut NavesApp, tool: &str) -> Option<&'a mut Option<PathBuf>> {
    let t = &mut app.prefs.tools;
    match tool {
        "armake2" => Some(&mut t.armake2),
        "Steam" => Some(&mut t.steam_root),
        "DayZ Tools" => Some(&mut t.dayz_tools),
        "DayZ" => Some(&mut t.dayz),
        "DayZ Server" => Some(&mut t.dayz_server),
        "Proton" => Some(&mut t.proton),
        _ => None,
    }
}

fn current_setting(app: &mut NavesApp, tool: &str) -> String {
    tool_slot(app, tool)
        .and_then(|s| s.as_ref())
        .map_or_else(String::new, |p| p.display().to_string())
}

fn set_tool_path(app: &mut NavesApp, tool: &str, text: &str) {
    if let Some(slot) = tool_slot(app, tool) {
        *slot = (!text.trim().is_empty()).then(|| PathBuf::from(text.trim()));
    }
}

impl NavesApp {
    /// Saves after a change of the shortcuts: the table of overrides goes into the settings.
    pub(super) fn save_shortcuts(&mut self) {
        self.save_prefs();
    }
}

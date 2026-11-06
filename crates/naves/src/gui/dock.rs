// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The bottom dock: a row of tabs and the tab you are in. A tab is one entry in [`TABS`].

use eframe::egui::{self, RichText, WidgetInfo};

use super::NavesApp;
use super::theme::{ERROR, GREEN};

/// A tab of the dock.
pub struct DockTabDef {
    /// Stable name.
    pub id: &'static str,
    /// What the tab says, with a count where it has one.
    pub label: fn(&NavesApp) -> String,
    /// What the tab shows.
    pub show: fn(&mut NavesApp, &mut egui::Ui),
}

/// The tabs, left to right. A later plan adds History or AI proposals here.
pub const TABS: &[DockTabDef] = &[
    DockTabDef {
        id: "build",
        label: build_label,
        show: build_tab,
    },
    DockTabDef {
        id: "problems",
        label: problems_label,
        show: problems_tab,
    },
];

fn build_label(_: &NavesApp) -> String {
    "Build".to_string()
}

fn build_tab(app: &mut NavesApp, ui: &mut egui::Ui) {
    app.build_ui(ui);
}

/// "Problems" or "Problems 7": no count when there is nothing to count.
pub fn problems_label(app: &NavesApp) -> String {
    match app.problems.shown(app.panel.active).len() {
        0 => "Problems".to_string(),
        n => format!("Problems {n}"),
    }
}

fn problems_tab(app: &mut NavesApp, ui: &mut egui::Ui) {
    let level = app.panel.active;
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.problems.all, "All");
    });
    let shown = app.problems.shown(level);
    if shown.is_empty() {
        ui.label("No problems");
        return;
    }
    let mut go_to = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for index in shown {
                let Some(problem) = app.problems.rows.get(index) else {
                    continue;
                };
                let selected = app.problems.cursor == Some(index);
                let text = RichText::new(&problem.message).color(if selected {
                    ui.visuals().text_color()
                } else {
                    ERROR
                });
                if ui.selectable_label(selected, text).clicked() {
                    go_to = Some(index);
                }
            }
        });
    if let Some(index) = go_to {
        app.go_to_problem(index);
    }
}

/// The row of tabs. The tab you are in is `app.dock`.
pub fn tab_row(ui: &mut egui::Ui, tabs: &[DockTabDef], app: &mut NavesApp) {
    ui.horizontal(|ui| {
        for tab in tabs {
            let label = (tab.label)(app);
            let selected = app.dock == tab.id;
            // a tab is its name with a bar under it when it is the one you are in; a filled button would be
            // mistaken for the Build button
            let text = if selected {
                RichText::new(&label).strong()
            } else {
                RichText::new(&label)
            };
            let response = ui.add(egui::Label::new(text).sense(egui::Sense::click()));
            if selected {
                let bar = egui::Rect::from_min_size(
                    egui::pos2(response.rect.left(), response.rect.bottom() + 1.0),
                    egui::vec2(response.rect.width(), 3.0),
                );
                ui.painter().rect_filled(bar, 0.0, GREEN);
            }
            if response.has_focus() {
                ui.painter().rect_stroke(
                    response.rect.expand(2.0),
                    2.0,
                    egui::Stroke::new(2.0, GREEN),
                    egui::StrokeKind::Outside,
                );
            }
            response.widget_info(|| {
                WidgetInfo::selected(egui::Role::Tab, true, selected, label.clone())
            });
            if response.clicked() {
                // the ids are 'static strings of the table
                app.dock = tab.id;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("Settings")
                .on_hover_text("These apply to every project")
                .clicked()
            {
                app.open_settings();
            }
        });
    });
}

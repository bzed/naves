// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The items a level can add: one model for all levels, so that one widget serves them all.
//!
//! No level has items yet, so the panel is not drawn; milestone 2 brings the first ones.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, RichText};

use super::NavesApp;
use super::levels::Level;
use super::shortcuts::ToolId;

/// How many items "Last used" holds per level.
const LAST_USED: usize = 5;

/// What an item looks like in the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Thumbnail {
    /// A colour: a surface.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the items of milestone 2 construct these")
    )]
    Swatch([u8; 3]),
    /// The line itself, at its width: a road class.
    #[expect(dead_code, reason = "the items of milestone 2 construct these")]
    Line {
        /// The width in metres.
        width_m: f64,
        /// The colour.
        rgb: [u8; 3],
    },
    /// A picture from the catalogue.
    #[expect(dead_code, reason = "the items of milestone 2 construct these")]
    Image(egui::TextureId),
}

/// What an item is. The list says so in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    /// Goes through the generators and can regenerate.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the items of milestone 2 construct these")
    )]
    Preset,
    /// A placed object that generators never touch.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the items of milestone 2 construct these")
    )]
    Model,
    /// A ground surface.
    #[expect(dead_code, reason = "the items of milestone 2 construct these")]
    Surface,
    /// A class of line: a road, a fence.
    #[expect(dead_code, reason = "the items of milestone 2 construct these")]
    LineClass,
    /// A group of things.
    #[expect(dead_code, reason = "the items of milestone 2 construct these")]
    Group,
}

impl ItemKind {
    fn word(self) -> &'static str {
        match self {
            ItemKind::Preset => "Preset",
            ItemKind::Model => "Single model",
            ItemKind::Surface => "Surface",
            ItemKind::LineClass => "Line class",
            ItemKind::Group => "Group",
        }
    }
}

/// Where an item comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemSource {
    /// The game's own.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the items of milestone 2 construct these")
    )]
    Game,
    /// A mod's.
    #[expect(dead_code, reason = "the items of milestone 2 construct these")]
    Mod,
    /// Made in this project.
    #[expect(dead_code, reason = "the items of milestone 2 construct these")]
    Project,
}

impl ItemSource {
    fn word(self) -> &'static str {
        match self {
            ItemSource::Game => "From the game",
            ItemSource::Mod => "From a mod",
            ItemSource::Project => "From this project",
        }
    }
}

/// One item of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteItem {
    /// Stable id.
    pub id: String,
    /// The name in the list.
    pub name: String,
    /// The group, in the level's own terms: style and use, biome, road class.
    pub group: String,
    /// The picture beside the name.
    pub thumbnail: Thumbnail,
    /// What it is.
    pub kind: ItemKind,
    /// The tool picking it switches to.
    pub default_tool: ToolId,
    /// Where it comes from.
    pub source: ItemSource,
}

/// The state of the list: the search text and the last used items of each level.
#[derive(Debug, Default)]
pub struct Palette {
    /// What is typed in the search field.
    pub search: String,
    /// The ids picked last, newest first.
    pub last_used: BTreeMap<Level, Vec<String>>,
    /// Ctrl+F was pressed: the search field takes the keyboard when it is drawn.
    pub focus_search: bool,
}

/// The items whose name or group contains `search`, ignoring case. All items for an empty search.
#[must_use]
pub fn filter<'a>(items: &'a [PaletteItem], search: &str) -> Vec<&'a PaletteItem> {
    let needle = search.trim().to_lowercase();
    items
        .iter()
        .filter(|i| {
            needle.is_empty()
                || i.name.to_lowercase().contains(&needle)
                || i.group.to_lowercase().contains(&needle)
        })
        .collect()
}

/// The list in the left panel: search, last used, then the groups in alphabetical order.
pub fn palette_ui(app: &mut NavesApp, ui: &mut egui::Ui, items: &[PaletteItem]) {
    let level = app.panel.active;
    let search = ui.add(
        egui::TextEdit::singleline(&mut app.palette.search)
            .hint_text("Search items")
            .desired_width(f32::INFINITY),
    );
    if std::mem::take(&mut app.palette.focus_search) {
        search.request_focus();
    }
    ui.add_space(6.0);
    let found = filter(items, &app.palette.search);
    if found.is_empty() {
        ui.label("Nothing matches.");
        return;
    }
    let mut picked = None;
    let mut row = |ui: &mut egui::Ui, item: &PaletteItem, selected: bool| {
        ui.horizontal(|ui| {
            thumbnail(ui, &item.thumbnail);
            if ui
                .selectable_label(selected, &item.name)
                .on_hover_text(item.source.word())
                .clicked()
            {
                picked = Some(item.id.clone());
            }
            ui.label(RichText::new(item.kind.word()).weak());
        });
    };
    if app.palette.search.trim().is_empty() {
        let recent: Vec<&PaletteItem> = app
            .palette
            .last_used
            .get(&level)
            .into_iter()
            .flatten()
            .filter_map(|id| items.iter().find(|i| &i.id == id))
            .collect();
        if !recent.is_empty() {
            ui.label(RichText::new("Last used").strong());
            for item in recent {
                row(ui, item, app.item.as_deref() == Some(&item.id));
            }
            ui.add_space(8.0);
        }
    }
    let mut groups: BTreeMap<&str, Vec<&PaletteItem>> = BTreeMap::new();
    for item in found {
        groups.entry(item.group.as_str()).or_default().push(item);
    }
    for (group, members) in groups {
        ui.label(RichText::new(group).strong());
        for item in members {
            row(ui, item, app.item.as_deref() == Some(&item.id));
        }
        ui.add_space(8.0);
    }
    if let Some(id) = picked
        && let Some(item) = items.iter().find(|i| i.id == id)
    {
        app.pick_item(item);
    }
}

fn thumbnail(ui: &mut egui::Ui, thumbnail: &Thumbnail) {
    let size = egui::vec2(24.0, 24.0);
    match thumbnail {
        Thumbnail::Swatch([r, g, b]) => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 2.0, Color32::from_rgb(*r, *g, *b));
        }
        Thumbnail::Line {
            width_m,
            rgb: [r, g, b],
        } => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            #[expect(clippy::cast_possible_truncation, reason = "a few pixels")]
            let width = width_m.clamp(1.0, 10.0) as f32;
            ui.painter().line_segment(
                [rect.left_center(), rect.right_center()],
                egui::Stroke::new(width, Color32::from_rgb(*r, *g, *b)),
            );
        }
        Thumbnail::Image(id) => {
            ui.image((*id, size));
        }
    }
}

impl NavesApp {
    /// Picking an item: remember it as last used and switch to its tool.
    pub(super) fn pick_item(&mut self, item: &PaletteItem) {
        let used = self.palette.last_used.entry(self.panel.active).or_default();
        used.retain(|id| id != &item.id);
        used.insert(0, item.id.clone());
        used.truncate(LAST_USED);
        self.item = Some(item.id.clone());
        self.tool = item.default_tool;
    }
}

// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The items a level can add: one model for all levels, so that one widget serves them all.
//!
//! The list has two scopes: what fits the active level, or everything. An item says which levels it fits; the
//! thumbnails share one scale (2 px per metre), so a 12 m road is visibly wider than a 5 m track.

use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, Color32};

use super::NavesApp;
use super::levels::Level;
use super::shortcuts::ToolId;
use super::theme::{self, tokens};

/// What the selector lists: the same type the settings file stores.
pub use naves_core::prefs::SelectorScope as Scope;

/// How many items "Last used" holds per level.
const LAST_USED: usize = 5;

/// The scale of every thumbnail: pixels per metre.
pub const THUMB_PX_PER_M: f32 = 2.0;
/// The width of the thumbnail column in pixels; what is longer is clipped.
pub const THUMB_COLUMN_PX: f32 = 64.0;
/// The height of a row, which the thickest thumbnail fills.
const ROW_PX: f32 = 24.0;
/// The side of the level chip.
const CHIP_PX: f32 = 14.0;
/// The width kept for the size at the right edge.
const SIZE_COLUMN_PX: f32 = 44.0;

/// What an item looks like in the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Thumbnail {
    /// A colour: a surface.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the items of milestone 2 construct these")
    )]
    Swatch([u8; 3]),
    /// The line itself, at its width on the shared scale: a road class.
    Line {
        /// The width in metres.
        width_m: f64,
        /// The colour.
        rgb: [u8; 3],
    },
    /// The plan view of a piece or a deck, in metres and centred: its outline at the shared scale.
    Footprint {
        /// The corners in metres, centred on the middle of the item.
        outline: Vec<(f32, f32)>,
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
    Model,
    /// A ground surface.
    #[expect(dead_code, reason = "the items of milestone 2 construct these")]
    Surface,
    /// A class of line: a road, a fence.
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
    /// The levels it fits; never empty. The first is the level its tools belong to.
    pub fits: Vec<Level>,
    /// Its width or length in metres, shown at the right edge.
    pub size_m: Option<f64>,
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
pub fn filter<'a>(
    items: impl IntoIterator<Item = &'a PaletteItem>,
    search: &str,
) -> Vec<&'a PaletteItem> {
    let needle = search.trim().to_lowercase();
    items
        .into_iter()
        .filter(|i| {
            needle.is_empty()
                || i.name.to_lowercase().contains(&needle)
                || i.group.to_lowercase().contains(&needle)
        })
        .collect()
}

/// The items of `scope` seen from `level`. `Level` keeps what fits the level, in the given order. `Everything` keeps
/// all: what fits the level first (its classes of line before its models), then by the first level an item fits (in stack order), the group and the name.
#[must_use]
pub fn in_scope(items: &[PaletteItem], level: Level, scope: Scope) -> Vec<&PaletteItem> {
    let fits = |i: &PaletteItem| i.fits.contains(&level);
    match scope {
        Scope::Level => items.iter().filter(|i| fits(i)).collect(),
        Scope::Everything => {
            let mut all: Vec<&PaletteItem> = items.iter().collect();
            all.sort_by(|a, b| {
                // the classes of line stay above the models of their level
                let key = |i: &PaletteItem| {
                    (
                        !fits(i),
                        i.kind != ItemKind::LineClass,
                        i.fits.iter().min().copied(),
                    )
                };
                key(a)
                    .cmp(&key(b))
                    .then_with(|| a.group.cmp(&b.group))
                    .then_with(|| a.name.cmp(&b.name))
            });
            all
        }
    }
}

/// What the list sorts by in the level's scope: the classes of line first, then the group.
fn list_order(item: &PaletteItem) -> (bool, &str) {
    let class = item.kind == ItemKind::LineClass;
    (!class, if class { "" } else { item.group.as_str() })
}

/// The size a thumbnail is drawn at, in pixels, on the one shared scale: at most the column wide and the row high.
#[must_use]
pub(super) fn thumbnail_size(thumbnail: &Thumbnail) -> egui::Vec2 {
    match thumbnail {
        Thumbnail::Swatch(_) => egui::vec2(16.0, 16.0),
        Thumbnail::Line { width_m, .. } => {
            #[expect(clippy::cast_possible_truncation, reason = "a few pixels")]
            let thickness = (*width_m as f32 * THUMB_PX_PER_M).clamp(1.0, ROW_PX);
            egui::vec2(THUMB_COLUMN_PX, thickness)
        }
        Thumbnail::Footprint { outline } => {
            let (mut min, mut max) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
            for &(x, y) in outline {
                min = (min.0.min(x), min.1.min(y));
                max = (max.0.max(x), max.1.max(y));
            }
            if outline.is_empty() {
                return egui::Vec2::ZERO;
            }
            egui::vec2(
                ((max.0 - min.0) * THUMB_PX_PER_M).clamp(1.0, THUMB_COLUMN_PX),
                ((max.1 - min.1) * THUMB_PX_PER_M).clamp(1.0, ROW_PX),
            )
        }
        Thumbnail::Image(_) => egui::vec2(ROW_PX, ROW_PX),
    }
}

/// The list in the left panel: search, the scope switch, last used, then the groups.
pub fn palette_ui(app: &mut NavesApp, ui: &mut egui::Ui) {
    let level = app.panel.active;
    let search = ui.add(
        egui::TextEdit::singleline(&mut app.palette.search)
            .hint_text("Search items")
            .desired_width(f32::INFINITY),
    );
    if std::mem::take(&mut app.palette.focus_search) {
        search.request_focus();
    }
    ui.add_space(4.0);
    let scope = app.selector_scope(level);
    if let Some(chosen) = scope_switch(ui, level, scope) {
        app.set_selector_scope(level, chosen);
    }
    ui.add_space(6.0);
    let scope = app.selector_scope(level);
    let all = app.all_items();
    let mut scoped = in_scope(&all, level, scope);
    if scope == Scope::Level {
        // a stable sort: the classes of line first, as the level gave them (the Roads level wants its classes above
        // hundreds of models); then the groups in alphabetical order, the items as the level gave them
        scoped.sort_by(|a, b| list_order(a).cmp(&list_order(b)));
    }
    let found = filter(scoped.iter().copied(), &app.palette.search);
    if found.is_empty() {
        ui.label("Nothing matches.");
        return;
    }
    let picked_id = app.item.clone();
    let mut picked = None;
    naves_ui::list_item::list_item_scope(ui, "palette", |ui| {
        let header = |ui: &mut egui::Ui, text: &str| {
            naves_ui::list_item::ListItem::new()
                .header()
                .show_flat(ui, naves_ui::list_item::LabelContent::header(text));
        };
        let mut row = |ui: &mut egui::Ui, item: &PaletteItem| {
            let selected = picked_id.as_deref() == Some(item.id.as_str());
            let chip = (!item.fits.contains(&level))
                .then(|| item.fits.first().copied())
                .flatten();
            let response = naves_ui::list_item::ListItem::new()
                .selected(selected)
                .show_flat(
                    ui,
                    naves_ui::list_item::CustomContent::new(|ui, context| {
                        item_row(ui, context.rect, item, chip);
                    }),
                )
                .on_hover_text(format!("{}. {}", item.kind.word(), item.source.word()));
            #[cfg(test)]
            if response.hovered() {
                app.hovered_row = Some(item.id.clone());
            }
            if response.clicked() {
                picked = Some(item.id.clone());
            }
        };
        if app.palette.search.trim().is_empty() {
            let recent: Vec<&PaletteItem> = app
                .palette
                .last_used
                .get(&level)
                .into_iter()
                .flatten()
                .filter_map(|id| scoped.iter().copied().find(|i| &i.id == id))
                .collect();
            if !recent.is_empty() {
                header(ui, "Last used");
                for item in recent {
                    row(ui, item);
                }
                ui.add_space(8.0);
            }
        }
        // consecutive items of one group share a header; the order is the scope's
        let mut current: Option<&str> = None;
        for item in found {
            if current != Some(item.group.as_str()) {
                if current.is_some() {
                    ui.add_space(8.0);
                }
                header(ui, &item.group);
                current = Some(item.group.as_str());
            }
            row(ui, item);
        }
        ui.add_space(8.0);
    });
    if let Some(id) = picked
        && let Some(item) = all.iter().find(|i| i.id == id)
    {
        app.pick_item(item);
    }
}

/// The two-segment switch under the search: the level's name and "Everything". Returns the new choice.
fn scope_switch(ui: &mut egui::Ui, level: Level, current: Scope) -> Option<Scope> {
    let mut chosen = None;
    let raised = if ui.visuals().dark_mode {
        tokens::RAISED
    } else {
        ui.visuals().faint_bg_color
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (scope, text) in [
            (Scope::Level, level.name()),
            (Scope::Everything, "Everything"),
        ] {
            let on = current == scope;
            let response = ui.add(
                egui::Button::selectable(on, text)
                    .fill(if on { raised } else { Color32::TRANSPARENT })
                    .stroke(egui::Stroke::NONE),
            );
            if on {
                // the chosen segment is also underlined, so that it does not rest on the fill alone
                let line = egui::Rect::from_min_max(
                    egui::pos2(response.rect.left(), response.rect.bottom() - 2.0),
                    response.rect.right_bottom(),
                );
                ui.painter()
                    .rect_filled(line, 0.0, theme::active(ui.visuals()));
            }
            if response.clicked() && !on {
                chosen = Some(scope);
            }
        }
    });
    chosen
}

/// One row: the thumbnail column, the name, and at the right the level chip and the size.
fn item_row(ui: &mut egui::Ui, rect: egui::Rect, item: &PaletteItem, chip: Option<Level>) {
    let right_w = SIZE_COLUMN_PX + if chip.is_some() { CHIP_PX + 4.0 } else { 0.0 };
    let thumb = egui::Rect::from_min_size(rect.min, egui::vec2(THUMB_COLUMN_PX, rect.height()));
    paint_thumbnail(ui, thumb, &item.thumbnail);
    let right = egui::Rect::from_min_max(
        egui::pos2((rect.right() - right_w).max(thumb.right()), rect.top()),
        rect.max,
    );
    let name = egui::Rect::from_min_max(
        egui::pos2(thumb.right() + 6.0, rect.top()),
        egui::pos2((right.left() - 4.0).max(thumb.right() + 6.0), rect.bottom()),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(name)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.add(
                egui::Label::new(item.name.as_str())
                    .truncate()
                    .selectable(false),
            );
            // what it is, in words, quiet beside the name
            ui.add(
                egui::Label::new(egui::RichText::new(item.kind.word()).color(tokens::INK_QUIET))
                    .truncate()
                    .selectable(false),
            );
        },
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(right)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            if let Some(size) = item.size_m {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("{size:.0} m")).color(tokens::INK_QUIET),
                    )
                    .selectable(false),
                );
            }
            if let Some(level) = chip {
                level_chip(ui, level);
            }
        },
    );
}

/// The chip of a level: its digit on its band colour; the level's name is its tooltip and its accessible label.
fn level_chip(ui: &mut egui::Ui, level: Level) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(CHIP_PX, CHIP_PX), egui::Sense::hover());
    ui.painter().rect_filled(rect, 2.0, level.colour());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        level.digit().to_string(),
        egui::FontId::proportional(10.0),
        level.band_text(),
    );
    let label = format!("{} {}", level.digit(), level.name());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label.clone()));
    response.on_hover_text(level.name());
}

/// Draws a thumbnail in the column `rect`, vertically centred: a colour, a line, an outline or a picture. What is
/// longer than the column is clipped and fades out.
fn paint_thumbnail(ui: &egui::Ui, rect: egui::Rect, thumbnail: &Thumbnail) {
    let painter = ui.painter().with_clip_rect(rect);
    let size = thumbnail_size(thumbnail);
    let ink = ui.visuals().text_color();
    let at_left = |size: egui::Vec2| {
        egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.center().y - size.y / 2.0),
            size,
        )
    };
    let mut clipped = false;
    match thumbnail {
        Thumbnail::Swatch([r, g, b]) => {
            painter.rect_filled(at_left(size), 2.0, Color32::from_rgb(*r, *g, *b));
        }
        Thumbnail::Line { rgb: [r, g, b], .. } => {
            painter.rect_filled(at_left(size), 0.0, Color32::from_rgb(*r, *g, *b));
            clipped = true; // a line has no end in the column
        }
        Thumbnail::Footprint { outline } => {
            let mid = at_left(size).center();
            let points: Vec<egui::Pos2> = outline
                .iter()
                .map(|&(x, y)| mid + egui::vec2(x, y) * THUMB_PX_PER_M)
                .collect();
            painter.add(egui::Shape::closed_line(
                points,
                egui::Stroke::new(1.5, ink),
            ));
            clipped = outline
                .iter()
                .any(|&(x, _)| x.abs() * THUMB_PX_PER_M > THUMB_COLUMN_PX / 2.0);
        }
        Thumbnail::Image(id) => {
            // painted like the design system paints its watermark: a mesh with the texture
            let mut mesh = egui::Mesh::with_texture(*id);
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            mesh.add_rect_with_uv(at_left(size), uv, Color32::WHITE);
            painter.add(egui::Shape::mesh(mesh));
        }
    }
    if clipped {
        // the last 12 px fade into the panel
        let ground = ui.visuals().panel_fill;
        for step in 0..4u8 {
            let x = rect.right() - 12.0 + f32::from(step) * 3.0;
            let strip = egui::Rect::from_min_max(
                egui::pos2(x, rect.top()),
                egui::pos2(x + 3.0, rect.bottom()),
            );
            painter.rect_filled(strip, 0.0, ground.gamma_multiply(f32::from(step + 1) * 0.2));
        }
    }
}

impl NavesApp {
    /// What the selector lists in `level`.
    pub(super) fn selector_scope(&self, level: Level) -> Scope {
        self.prefs
            .map
            .selector_scope
            .get(level.id())
            .copied()
            .unwrap_or_default()
    }

    /// Remembers the scope of `level` in the settings.
    pub(super) fn set_selector_scope(&mut self, level: Level, scope: Scope) {
        let scopes = &mut self.prefs.map.selector_scope;
        if scope == Scope::Level {
            scopes.remove(level.id());
        } else {
            scopes.insert(level.id().to_string(), scope);
        }
        self.save_prefs();
    }

    /// The items of every built level, each id once. (Catalogue models are in the Roads level's own items too.)
    pub(super) fn all_items(&self) -> Vec<PaletteItem> {
        let mut seen = BTreeSet::new();
        let mut all = Vec::new();
        for def in &self.levels {
            for item in (def.items)(self) {
                if seen.insert(item.id.clone()) {
                    all.push(item);
                }
            }
        }
        for item in self.models() {
            if seen.insert(item.id.clone()) {
                all.push(item);
            }
        }
        all
    }

    /// Picking an item: remember it as last used and switch to its tool. An item of another level keeps the active
    /// level; its tools join the rail until Esc.
    pub(super) fn pick_item(&mut self, item: &PaletteItem) {
        let used = self.palette.last_used.entry(self.panel.active).or_default();
        used.retain(|id| id != &item.id);
        used.insert(0, item.id.clone());
        used.truncate(LAST_USED);
        self.item = Some(item.id.clone());
        self.item_level = if item.fits.contains(&self.panel.active) {
            None
        } else {
            item.fits.first().copied()
        };
        self.tool = item.default_tool;
        if item.kind == ItemKind::Model {
            self.selection = None; // the card of the model takes the properties
        }
    }
}

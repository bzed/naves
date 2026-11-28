// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The right-hand properties of what the editor holds: the card of a catalogue model and the properties of a
//! selected way.

use std::collections::BTreeMap;

use eframe::egui::{self, RichText};

use super::{EditSession, Origin};
use crate::gui::levels::roads::{ROAD_CLASSES, RoadClass};
use crate::gui::palette::PaletteItem;
use crate::gui::theme::tokens;

/// The surfaces the combo offers, in the order of the combo.
pub const SURFACES: &[&str] = &["asphalt", "paved", "gravel", "compacted", "dirt", "grass"];

/// The tags a class sets that no other class shares by name: removed when the class changes.
const CLASS_KEYS: &[&str] = &["highway", "tracktype"];

/// A change of one property of a way. Each is one edit, so one undo step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WayChange {
    /// The class (an id of [`ROAD_CLASSES`]).
    Class(&'static str),
    /// The surface; `None` removes the tag.
    Surface(Option<&'static str>),
    /// On or off the `bridge` tag.
    Bridge(bool),
    /// The `layer` tag, limited to -5 to 5; 0 removes the tag.
    Layer(i32),
    /// `naves:keep`: the way stays as drawn.
    Keep(bool),
    /// `naves:plan`: road planning may route the way.
    Plan(bool),
    /// Removes the edit of an OpenStreetMap way.
    Reset,
}

/// Where a way comes from, for the last line of the properties.
#[must_use]
pub fn origin_line(origin: Origin, raw: i64) -> String {
    match origin {
        Origin::Osm => format!("OSM way {raw}, unchanged"),
        Origin::EditedOsm => format!("OSM way {raw}, edited by you"),
        Origin::Drawn => "Drawn by you".to_string(),
    }
}

/// The class a way's tags make: the matching one with the most tags (a grade 2 track is not a plain track).
#[must_use]
pub fn class_of(tags: &BTreeMap<String, String>) -> Option<&'static RoadClass> {
    ROAD_CLASSES
        .iter()
        .filter(|c| {
            c.tags
                .iter()
                .all(|(k, v)| tags.get(*k).is_some_and(|t| t == v))
        })
        .max_by_key(|c| c.tags.len())
}

/// The class in the combo: its name, or `Other: highway=...`.
#[must_use]
pub fn class_label(tags: &BTreeMap<String, String>) -> String {
    match (class_of(tags), tags.get("highway")) {
        (Some(class), _) => class.name.to_string(),
        (None, Some(value)) => format!("Other: highway={value}"),
        (None, None) => "Other".to_string(),
    }
}

/// The title of a way: the name of its class, else "Road".
#[must_use]
pub fn way_title(tags: &BTreeMap<String, String>) -> String {
    class_of(tags).map_or_else(|| "Road".to_string(), |c| c.name.to_string())
}

fn set_or_remove(tags: &mut BTreeMap<String, String>, key: &str, value: Option<String>) {
    match value {
        Some(value) => {
            tags.insert(key.to_string(), value);
        }
        None => {
            tags.remove(key);
        }
    }
}

/// The tags after `change`; `None` when the change names no class or is a reset.
#[must_use]
pub fn changed_tags(
    tags: &BTreeMap<String, String>,
    change: &WayChange,
) -> Option<BTreeMap<String, String>> {
    let mut tags = tags.clone();
    match change {
        WayChange::Class(id) => {
            let new = ROAD_CLASSES.iter().find(|c| c.id == *id)?;
            // the old class's tags go, every other tag stays
            for key in CLASS_KEYS {
                tags.remove(*key);
            }
            for (k, v) in new.tags {
                tags.insert((*k).to_string(), (*v).to_string());
            }
        }
        WayChange::Surface(surface) => {
            set_or_remove(&mut tags, "surface", surface.map(str::to_string));
        }
        WayChange::Bridge(on) => {
            if *on {
                tags.insert("bridge".to_string(), "yes".to_string());
                tags.entry("layer".to_string())
                    .or_insert_with(|| "1".to_string());
            } else {
                // the same as the Bridge tool: the road is a plain road again
                tags.remove("bridge");
                tags.remove("layer");
            }
        }
        WayChange::Layer(layer) => {
            let layer = (*layer).clamp(-5, 5);
            set_or_remove(&mut tags, "layer", (layer != 0).then(|| layer.to_string()));
        }
        WayChange::Keep(on) => {
            let value = if *on { "yes" } else { "no" };
            tags.insert("naves:keep".to_string(), value.to_string());
        }
        WayChange::Plan(on) => {
            set_or_remove(&mut tags, "naves:plan", on.then(|| "yes".to_string()));
        }
        WayChange::Reset => return None,
    }
    Some(tags)
}

/// The properties of the way `raw` (and of the `node` that was picked on it): the title, the class, surface,
/// bridge, layer and the two switches, the origin and, for an edited OpenStreetMap way, the reset. Returns the
/// change the person made, if any.
pub fn way_properties(
    ui: &mut egui::Ui,
    session: &EditSession,
    raw: i64,
    node: Option<i64>,
) -> Option<WayChange> {
    let way = session.way(raw)?;
    let origin = session.origin(raw);
    let tags = &way.tags;
    let mut change = None;
    ui.add_space(14.0);
    ui.label(RichText::new(way_title(tags)).strong());
    ui.label(RichText::new(origin_line(origin, raw)).color(tokens::INK_QUIET));
    if let Some(name) = tags.get("name") {
        ui.label(name);
    }
    if let Some(node) = node {
        ui.label(RichText::new(format!("Node {node}")).color(tokens::INK_QUIET));
    }
    ui.add_space(6.0);
    egui::Grid::new("way-properties")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label("Class");
            egui::ComboBox::from_id_salt("way-class")
                .selected_text(class_label(tags))
                .show_ui(ui, |ui| {
                    let current = class_of(tags).map(|c| c.id);
                    for class in ROAD_CLASSES {
                        if ui
                            .selectable_label(current == Some(class.id), class.name)
                            .clicked()
                        {
                            change = Some(WayChange::Class(class.id));
                        }
                    }
                });
            ui.end_row();

            ui.label("Surface");
            let surface = tags.get("surface").map(String::as_str);
            egui::ComboBox::from_id_salt("way-surface")
                .selected_text(surface.unwrap_or("—"))
                .show_ui(ui, |ui| {
                    for value in SURFACES {
                        if ui
                            .selectable_label(surface == Some(*value), *value)
                            .clicked()
                        {
                            change = Some(WayChange::Surface(Some(value)));
                        }
                    }
                    if ui.selectable_label(surface.is_none(), "—").clicked() {
                        change = Some(WayChange::Surface(None));
                    }
                });
            ui.end_row();

            ui.label("Bridge");
            let mut bridge = tags.get("bridge").is_some_and(|b| b != "no");
            if ui.checkbox(&mut bridge, "").changed() {
                change = Some(WayChange::Bridge(bridge));
            }
            ui.end_row();

            ui.label("Layer");
            // the value being dragged or typed is kept until the drag ends or the field is left, so that
            // one gesture is one edit
            let memory = ui.make_persistent_id(("way-layer", raw));
            let held: Option<i32> = ui.data(|d| d.get_temp(memory));
            let mut layer: i32 = held.unwrap_or_else(|| {
                tags.get("layer")
                    .and_then(|l| l.parse().ok())
                    .unwrap_or(0)
                    .clamp(-5, 5)
            });
            let response = ui.add(egui::DragValue::new(&mut layer).range(-5..=5).speed(0.05));
            if response.changed() {
                ui.data_mut(|d| d.insert_temp(memory, layer));
            }
            if response.drag_stopped() || response.lost_focus() {
                ui.data_mut(|d| d.remove_temp::<i32>(memory));
                if held.is_some() || response.changed() {
                    change = Some(WayChange::Layer(layer));
                }
            } else if held.is_some() && !response.dragged() && !response.has_focus() {
                ui.data_mut(|d| d.remove_temp::<i32>(memory)); // the gesture was abandoned
            }
            ui.end_row();
        });
    let mut keep = tags.get("naves:keep").is_some_and(|v| v == "yes");
    if ui.checkbox(&mut keep, "Keep as drawn").changed() {
        change = Some(WayChange::Keep(keep));
    }
    let mut plan = tags.get("naves:plan").is_some_and(|v| v == "yes");
    if ui
        .checkbox(&mut plan, "Let road planning route it")
        .changed()
    {
        change = Some(WayChange::Plan(plan));
    }
    if origin == Origin::EditedOsm {
        ui.add_space(6.0);
        if ui.button("Reset to OSM").clicked() {
            change = Some(WayChange::Reset);
        }
    }
    change
}

/// The levels an item fits, in words: "Roads and paths".
fn fits_text(item: &PaletteItem) -> String {
    let names: Vec<&str> = item.fits.iter().map(|l| l.name()).collect();
    names.join(", ")
}

/// The card of a model picked in the list: its name, size and levels, and what it cannot do yet.
pub fn model_card(ui: &mut egui::Ui, item: &PaletteItem) {
    ui.add_space(14.0);
    ui.label(RichText::new(&item.name).strong());
    egui::Grid::new("model-card")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            if let Some(size) = item.size_m {
                ui.label("Size");
                ui.label(format!("{size:.0} m"));
                ui.end_row();
            }
            ui.label("Fits");
            ui.label(fits_text(item));
            ui.end_row();
        });
    ui.add_space(4.0);
    ui.label(RichText::new("Placing models comes in a later version.").color(tokens::INK_QUIET));
}

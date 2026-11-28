// SPDX-FileCopyrightText: 2026 Bernd Zeimetz <bernd@bzed.de>
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The right-hand properties of what the editor holds: the card of a catalogue model.

use eframe::egui::{self, RichText};

use crate::gui::palette::PaletteItem;
use crate::gui::theme::tokens;

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

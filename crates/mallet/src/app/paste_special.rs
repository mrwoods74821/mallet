//! Hammer's Paste Special dialog (Ctrl+Shift+V): paste several copies of the clipboard at
//! once, each moved and turned from the one before, with renamed entities and grouping.

use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as icon;
use glam::DVec3;

use super::MalletApp;
use crate::editor::{Editor, PasteOptions, MAX_COPIES};

/// Three number fields for X, Y and Z on one grid row.
fn vector_row(ui: &mut egui::Ui, label: &str, value: &mut DVec3, speed: f64) {
    ui.label(label);
    ui.horizontal(|ui| {
        for (axis, number) in ["X", "Y", "Z"].iter().zip(value.as_mut()) {
            ui.label(RichText::new(*axis).color(Color32::GRAY));
            ui.add(egui::DragValue::new(number).speed(speed).max_decimals(4));
        }
    });
    ui.end_row();
}

impl MalletApp {
    /// Draws the Paste Special dialog while it is open.
    pub(super) fn paste_special_window(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        if !editor.paste_special_open {
            return;
        }
        let options = &mut self.paste_options;
        let mut open = true;
        let mut paste = false;
        let mut close = false;
        egui::Window::new(format!("{} Paste Special", icon::CLIPBOARD_TEXT))
            .id(egui::Id::new("paste_special_dialog"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_pos(ctx.screen_rect().center() - egui::vec2(170.0, 150.0))
            .show(ctx, |ui| {
                let count = editor.clipboard.len();
                let noun = if count == 1 { "object" } else { "objects" };
                ui.label(RichText::new(format!("{count} {noun} on the clipboard")).color(Color32::GRAY));
                ui.add_space(4.0);
                egui::Grid::new("paste_special_values").num_columns(2).show(ui, |ui| {
                    ui.label("Copies");
                    ui.add(egui::DragValue::new(&mut options.copies).range(1..=MAX_COPIES));
                    ui.end_row();
                    vector_row(ui, "Offset", &mut options.offset, 1.0);
                    vector_row(ui, "Rotation", &mut options.rotation, 1.0);
                });
                ui.label(
                    RichText::new("Each copy moves and turns this much more than the one before it.")
                        .small()
                        .color(Color32::GRAY),
                );
                ui.add_space(4.0);
                ui.checkbox(&mut options.start_at_original, "Start at the original")
                    .on_hover_text("Count the offset from where the objects were copied, not from the cursor");
                let has_pivot = editor.clipboard.pivot.is_some();
                ui.add_enabled(has_pivot, egui::Checkbox::new(&mut options.around_pivot, "Rotate around the pivot"))
                    .on_hover_text("Turn the copies around the pivot placed when they were copied, not their centre")
                    .on_disabled_hover_text("Place a pivot with the rotation handles before copying");
                ui.checkbox(&mut options.group, "Group each copy");
                ui.separator();
                ui.label("Entity names");
                egui::Grid::new("paste_special_names").num_columns(2).show(ui, |ui| {
                    ui.label("Prefix");
                    ui.text_edit_singleline(&mut options.prefix);
                    ui.end_row();
                    ui.label("Suffix");
                    ui.text_edit_singleline(&mut options.suffix);
                    ui.end_row();
                });
                ui.checkbox(&mut options.unique_names, "Make names unique")
                    .on_hover_text("Number pasted names so no other entity in the map has them. Outputs between pasted entities follow the new names.");
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Paste").clicked() {
                        paste = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if ui.button("Reset").on_hover_text("Back to a single copy with no offset, turn or renaming").clicked() {
                        *options = PasteOptions::default();
                    }
                });
            });
        if paste {
            let options = if editor.clipboard.pivot.is_some() {
                options.clone()
            } else {
                PasteOptions {
                    around_pivot: false,
                    ..options.clone()
                }
            };
            match editor.paste(editor.paste_special_at, &options) {
                Ok(_) => close = true,
                Err(error) => editor.message = error,
            }
        }
        if !open || close {
            editor.paste_special_open = false;
        }
    }
}

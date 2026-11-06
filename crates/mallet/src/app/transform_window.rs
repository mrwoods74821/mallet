//! Hammer's Transform dialog (Ctrl+M): rotate, scale or move the selection by typed
//! amounts, around the selection's centre, the placed pivot or the world origin.

use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as icon;
use glam::DVec3;

use super::MalletApp;
use crate::editor::Editor;
use crate::transform::Pivot;

/// What the dialog's three numbers mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Rotate,
    Scale,
    Move,
}

impl Mode {
    const ALL: [Mode; 3] = [Self::Rotate, Self::Scale, Self::Move];

    fn label(self) -> &'static str {
        match self {
            Self::Rotate => "Rotate",
            Self::Scale => "Scale",
            Self::Move => "Move",
        }
    }

    /// The numbers the dialog starts with in this mode: nothing changes.
    fn identity(self) -> [f64; 3] {
        match self {
            Self::Scale => [1.0; 3],
            Self::Rotate | Self::Move => [0.0; 3],
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Rotate => "Degrees around each axis, applied X, then Y, then Z.",
            Self::Scale => "A factor per axis; 2 doubles, 0.5 halves, -1 mirrors.",
            Self::Move => "Units along each axis.",
        }
    }
}

/// Where the Transform dialog turns and scales around.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Around {
    Center,
    Pivot,
    Origin,
}

/// What the Transform dialog keeps between frames.
pub(super) struct TransformState {
    mode: Mode,
    values: [f64; 3],
    around: Around,
}

impl Default for TransformState {
    fn default() -> Self {
        Self {
            mode: Mode::Rotate,
            values: Mode::Rotate.identity(),
            around: Around::Center,
        }
    }
}

impl MalletApp {
    /// Draws the Transform dialog while it is open.
    pub(super) fn transform_window(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        if !editor.transform_open {
            return;
        }
        let state = &mut self.transform;
        let mut open = true;
        let mut apply = false;
        let mut close = false;
        egui::Window::new(format!("{} Transform", icon::ARROWS_OUT_CARDINAL))
            .id(egui::Id::new("transform_dialog"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_pos(ctx.screen_rect().center() - egui::vec2(150.0, 120.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for mode in Mode::ALL {
                        if ui.selectable_label(state.mode == mode, mode.label()).clicked() && state.mode != mode {
                            state.mode = mode;
                            state.values = mode.identity();
                        }
                    }
                });
                ui.label(RichText::new(state.mode.hint()).small().color(Color32::GRAY));
                ui.add_space(4.0);
                egui::Grid::new("transform_values").num_columns(2).show(ui, |ui| {
                    for (axis, value) in ["X", "Y", "Z"].iter().zip(state.values.iter_mut()) {
                        ui.label(*axis);
                        let speed = if state.mode == Mode::Scale { 0.01 } else { 1.0 };
                        ui.add(egui::DragValue::new(value).speed(speed).max_decimals(4));
                        ui.end_row();
                    }
                });
                if state.mode != Mode::Move {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Around");
                        ui.selectable_value(&mut state.around, Around::Center, "Selection centre");
                        ui.selectable_value(&mut state.around, Around::Pivot, "Pivot")
                            .on_hover_text("The pivot placed with the rotation handles in a 2D view");
                        ui.selectable_value(&mut state.around, Around::Origin, "World origin");
                    });
                }
                ui.add_space(2.0);
                ui.checkbox(&mut editor.config.transform.texture_lock, "Texture lock");
                if state.mode == Mode::Scale {
                    ui.checkbox(&mut editor.config.transform.texture_scale_lock, "Texture scale lock");
                }
                ui.separator();
                ui.horizontal(|ui| {
                    let usable = !editor.selection.is_empty();
                    if ui.add_enabled(usable, egui::Button::new("OK")).clicked() {
                        apply = true;
                        close = true;
                    }
                    if ui
                        .add_enabled(usable, egui::Button::new("Apply"))
                        .on_hover_text("Apply and keep the dialog open")
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if !usable {
                        ui.label(RichText::new("Select something first").color(Color32::GRAY));
                    }
                });
            });
        if apply && state.values != state.mode.identity() {
            let values = DVec3::from_array(state.values);
            let pivot = match state.around {
                Around::Center => Pivot::Center,
                Around::Pivot => Pivot::Placed,
                Around::Origin => Pivot::Origin,
            };
            let result = match state.mode {
                Mode::Rotate => editor.rotate_selection(values, pivot),
                Mode::Scale => editor.scale_selection(values, pivot),
                Mode::Move => editor.move_selection(values),
            };
            if let Err(error) = result {
                editor.message = error;
                close = false;
            }
        }
        if !open || close {
            editor.transform_open = false;
            editor.save_config();
        }
    }
}

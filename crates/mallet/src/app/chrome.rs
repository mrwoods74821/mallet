//! The window chrome around the viewports, laid out like Hammer++: an icon toolbar under
//! the menus, browser-style map tabs, the tool palette on the left and the object panel on
//! the right.

use std::collections::HashMap;

use eframe::egui::{self, Color32, RichText, Sense, Vec2};
use egui_phosphor::regular as icon;

use super::{run, save_as_dialog, MalletApp};
use crate::editor::{Editor, Tool, ENTITY_CLASSES};
use crate::keymap::Action;

const TOOLBAR_ICON_SIZE: f32 = 17.0;
const PALETTE_ICON_SIZE: f32 = 21.0;
const PALETTE_WIDTH: f32 = 46.0;
const SIDE_PANEL_WIDTH: f32 = 236.0;
const TEXTURE_PREVIEW_SIZE: f32 = 96.0;
const ACTIVE_TAB: Color32 = Color32::from_gray(58);
const INACTIVE_TAB: Color32 = Color32::from_gray(36);

/// Previews of materials for the side panel, by material name. `None` when the material
/// has no texture.
pub(super) type TexturePreviews = HashMap<String, Option<(egui::TextureHandle, [u32; 2])>>;

/// A tool palette entry: its icon, name, and the tool and shortcut action when it exists.
type PaletteEntry = (&'static str, &'static str, Option<(Tool, Action)>);

/// Hammer's tool palette, top to bottom. Tools Mallet does not have yet are shown greyed
/// out so the layout already matches.
const PALETTE: [PaletteEntry; 9] = [
    (
        icon::CURSOR,
        "Selection tool",
        Some((Tool::Select, Action::SelectionTool)),
    ),
    (icon::LIGHTBULB, "Entity tool", Some((Tool::Entity, Action::EntityTool))),
    (icon::CUBE, "Block tool", Some((Tool::Block, Action::BlockTool))),
    (icon::PAINT_BRUSH, "Texture application", None),
    (icon::STAMP, "Apply decals", None),
    (icon::STACK, "Overlays", None),
    (icon::SCISSORS, "Clipping tool", None),
    (icon::POLYGON, "Vertex tool", None),
    (icon::PATH, "Path tool", None),
];

impl MalletApp {
    /// The icon toolbar under the menus.
    pub(super) fn toolbar(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut chosen = None;
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let groups: [&[(&str, &str, Action)]; 3] = [
                    &[
                        (icon::FILE_PLUS, "New map", Action::NewFile),
                        (icon::FOLDER_OPEN, "Open map", Action::OpenFile),
                        (icon::FLOPPY_DISK, "Save map", Action::SaveFile),
                    ],
                    &[
                        (icon::ARROW_U_UP_LEFT, "Undo", Action::Undo),
                        (icon::ARROW_U_UP_RIGHT, "Redo", Action::Redo),
                    ],
                    &[
                        (icon::CROSSHAIR, "Center 2D views on the map", Action::CenterViews2D),
                        (icon::CUBE_FOCUS, "Center the 3D view on the map", Action::CenterView3D),
                    ],
                ];
                for group in groups {
                    for (glyph, name, action) in group {
                        if self
                            .icon_button(ui, glyph, name, Some(*action), false, TOOLBAR_ICON_SIZE)
                            .clicked()
                        {
                            chosen = Some(*action);
                        }
                    }
                    ui.separator();
                }
                if self
                    .icon_button(
                        ui,
                        icon::MINUS,
                        "Smaller grid",
                        Some(Action::GridSmaller),
                        false,
                        TOOLBAR_ICON_SIZE,
                    )
                    .clicked()
                {
                    chosen = Some(Action::GridSmaller);
                }
                ui.add_sized(
                    [46.0, 26.0],
                    egui::Label::new(format!("{} {}", icon::GRID_FOUR, editor.grid)),
                )
                .on_hover_text("Grid size");
                if self
                    .icon_button(
                        ui,
                        icon::PLUS,
                        "Larger grid",
                        Some(Action::GridLarger),
                        false,
                        TOOLBAR_ICON_SIZE,
                    )
                    .clicked()
                {
                    chosen = Some(Action::GridLarger);
                }
                ui.separator();
                if self
                    .icon_button(
                        ui,
                        icon::GAUGE,
                        "Net graph",
                        None,
                        self.show_net_graph,
                        TOOLBAR_ICON_SIZE,
                    )
                    .clicked()
                {
                    self.show_net_graph = !self.show_net_graph;
                }
            });
            ui.add_space(2.0);
        });
        if let Some(action) = chosen {
            self.dispatch(action, editor);
        }
    }

    /// Browser-style tabs, one per open map.
    pub(super) fn tab_strip(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut switch = None;
        let mut close = None;
        let mut open_new = false;
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for (index, tab) in editor.tabs().iter().enumerate() {
                    let active = index == editor.active_tab();
                    let title = if tab.dirty {
                        format!("{} \u{2022}", tab.title)
                    } else {
                        tab.title.clone()
                    };
                    let text_color = if active {
                        Color32::WHITE
                    } else {
                        Color32::from_gray(170)
                    };
                    egui::Frame::default()
                        .fill(if active { ACTIVE_TAB } else { INACTIVE_TAB })
                        .corner_radius(4)
                        .inner_margin(egui::Margin::symmetric(8, 3))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let label = ui
                                    .add(egui::Label::new(RichText::new(title).color(text_color)).sense(Sense::click()))
                                    .on_hover_text(&tab.path);
                                if label.clicked() {
                                    switch = Some(index);
                                }
                                if label.middle_clicked() {
                                    close = Some(index);
                                }
                                let close_button = egui::Button::new(RichText::new(icon::X).size(12.0)).frame(false);
                                if ui.add(close_button).on_hover_text("Close tab (Ctrl+W)").clicked() {
                                    close = Some(index);
                                }
                            });
                        });
                }
                let new_button = egui::Button::new(RichText::new(icon::PLUS).size(14.0)).frame(false);
                if ui.add(new_button).on_hover_text("New map (Ctrl+N)").clicked() {
                    open_new = true;
                }
            });
            ui.add_space(3.0);
        });
        if let Some(index) = switch {
            let _ = editor.switch_tab(index);
        }
        if let Some(index) = close {
            self.request_close(index, editor);
        }
        if open_new {
            editor.new_tab();
        }
    }

    /// The vertical tool palette on the left.
    pub(super) fn tool_palette(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut chosen = None;
        egui::SidePanel::left("tool_palette")
            .resizable(false)
            .exact_width(PALETTE_WIDTH)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for (glyph, name, tool) in PALETTE {
                        match tool {
                            Some((tool, action)) => {
                                let selected = editor.tool == tool;
                                if self
                                    .icon_button(ui, glyph, name, Some(action), selected, PALETTE_ICON_SIZE)
                                    .clicked()
                                {
                                    chosen = Some(action);
                                }
                            }
                            None => {
                                let button = egui::Button::new(RichText::new(glyph).size(PALETTE_ICON_SIZE))
                                    .min_size(Vec2::splat(34.0));
                                ui.add_enabled(false, button)
                                    .on_disabled_hover_text(format!("{name} (coming soon)"));
                            }
                        }
                    }
                });
            });
        if let Some(action) = chosen {
            self.dispatch(action, editor);
        }
    }

    /// The object panel on the right: selection mode, current texture and selection.
    pub(super) fn side_panel(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        egui::SidePanel::right("side_panel")
            .resizable(true)
            .default_width(SIDE_PANEL_WIDTH)
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("Select").strong());
                ui.horizontal(|ui| {
                    let _ = ui
                        .add(egui::Button::new("Groups").selected(true))
                        .on_hover_text("Clicking part of an entity selects the whole entity");
                    for mode in ["Objects", "Solids"] {
                        ui.add_enabled(false, egui::Button::new(mode))
                            .on_disabled_hover_text(format!("{mode} selection mode (coming soon)"));
                    }
                });
                ui.separator();

                ui.label(RichText::new("Current texture").strong());
                ui.add(egui::TextEdit::singleline(&mut editor.material).desired_width(f32::INFINITY))
                    .on_hover_text("Material for new blocks");
                ui.horizontal(|ui| {
                    let preview = texture_preview(ctx, &mut self.texture_previews, editor);
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(TEXTURE_PREVIEW_SIZE), Sense::hover());
                    match &preview {
                        Some((texture, _)) => {
                            let full = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
                            ui.painter().image(texture.id(), rect, full, Color32::WHITE);
                        }
                        None => {
                            ui.painter().rect_filled(rect, 2.0, Color32::from_gray(45));
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "no preview",
                                egui::FontId::proportional(11.0),
                                Color32::GRAY,
                            );
                        }
                    }
                    ui.vertical(|ui| {
                        if let Some((_, [width, height])) = &preview {
                            ui.label(format!("{width}x{height}"));
                        }
                        ui.add_enabled(
                            false,
                            egui::Button::new(format!("{} Browse...", icon::MAGNIFYING_GLASS)),
                        )
                        .on_disabled_hover_text("Texture browser (coming soon)");
                    });
                });
                ui.separator();

                ui.label(RichText::new("New entity").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut editor.entity_class)
                        .desired_width(f32::INFINITY)
                        .hint_text("classname"),
                )
                .on_hover_text("Class the entity tool creates");
                ui.menu_button(format!("{} Common classes", icon::LIST), |ui| {
                    for class in ENTITY_CLASSES {
                        if ui.selectable_label(editor.entity_class == *class, *class).clicked() {
                            editor.entity_class = (*class).to_owned();
                            ui.close_menu();
                        }
                    }
                });
                ui.label(
                    RichText::new("Entity tool: click a surface in 3D, or place in 2D and press Enter")
                        .small()
                        .color(Color32::GRAY),
                );
                ui.separator();

                ui.label(RichText::new("Selection").strong());
                let bounds = editor.scene.selection_bounds();
                if bounds.is_empty() {
                    ui.label(RichText::new("Nothing selected").color(Color32::GRAY));
                } else {
                    let (size, centre) = (bounds.size(), bounds.center());
                    ui.label(format!("{} selected", editor.selection.len()));
                    ui.label(format!(
                        "Size {} x {} x {}",
                        short(size.x),
                        short(size.y),
                        short(size.z)
                    ));
                    ui.label(format!(
                        "Centre {} {} {}",
                        short(centre.x),
                        short(centre.y),
                        short(centre.z)
                    ));
                }
                ui.separator();

                egui::CollapsingHeader::new(RichText::new("VisGroups").strong())
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new("Coming soon").color(Color32::GRAY));
                    });
            });
    }

    /// Asks before closing a tab with unsaved changes; closes it straight away otherwise.
    pub(super) fn request_close(&mut self, index: usize, editor: &mut Editor) {
        match editor.tabs().get(index) {
            Some(tab) if tab.dirty => self.closing = Some(index),
            Some(_) => {
                let _ = editor.close_tab(index);
            }
            None => {}
        }
    }

    /// The "unsaved changes" question shown while a dirty tab is being closed.
    pub(super) fn close_dialog(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let Some(index) = self.closing else { return };
        let Some(tab) = editor.tabs().into_iter().nth(index) else {
            self.closing = None;
            return;
        };
        let mut answer = None;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!("Save changes to {} before closing it?", tab.title));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for choice in ["Save", "Don't save", "Cancel"] {
                        if ui.button(choice).clicked() {
                            answer = Some(choice);
                        }
                    }
                });
            });
        match answer {
            Some("Save") => {
                self.closing = None;
                if editor.switch_tab(index).is_err() {
                    return;
                }
                if editor.is_untitled() {
                    save_as_dialog(editor);
                } else {
                    let _ = editor.save(None);
                }
                if !editor.tabs()[editor.active_tab()].dirty {
                    let _ = editor.close_tab(editor.active_tab());
                }
            }
            Some("Don't save") => {
                self.closing = None;
                let _ = editor.close_tab(index);
            }
            Some(_) => self.closing = None,
            None => {}
        }
    }

    /// Runs an action, handling the ones that need the window's own state.
    pub(super) fn dispatch(&mut self, action: Action, editor: &mut Editor) {
        match action {
            Action::CloseTab => self.request_close(editor.active_tab(), editor),
            Action::NextTab | Action::PreviousTab => {
                let count = editor.tabs().len();
                let step = if action == Action::NextTab { 1 } else { count - 1 };
                let _ = editor.switch_tab((editor.active_tab() + step) % count);
            }
            _ => run(action, editor),
        }
    }

    /// A square icon button with a tooltip naming the action and its shortcut.
    fn icon_button(
        &self,
        ui: &mut egui::Ui,
        glyph: &str,
        name: &str,
        action: Option<Action>,
        selected: bool,
        size: f32,
    ) -> egui::Response {
        let shortcut = action.map(|action| self.keymap.label(action)).unwrap_or_default();
        let hint = if shortcut.is_empty() {
            name.to_owned()
        } else {
            format!("{name} ({shortcut})")
        };
        let button = egui::Button::new(RichText::new(glyph).size(size))
            .selected(selected)
            .min_size(Vec2::splat(size + 12.0));
        ui.add(button).on_hover_text(hint)
    }
}

/// The preview texture of the current material, loaded once per material name.
fn texture_preview(
    ctx: &egui::Context,
    previews: &mut TexturePreviews,
    editor: &mut Editor,
) -> Option<(egui::TextureHandle, [u32; 2])> {
    let name = editor.material.trim().to_owned();
    if let Some(cached) = previews.get(&name) {
        return cached.clone();
    }
    let image = editor.materials.as_mut()?.texture(&name);
    let preview = image.map(|image| {
        let pixels = egui::ColorImage::from_rgba_unmultiplied(
            [image.pixel_width as usize, image.pixel_height as usize],
            &image.rgba,
        );
        let handle = ctx.load_texture(format!("material:{name}"), pixels, egui::TextureOptions::LINEAR);
        (handle, [image.width, image.height])
    });
    previews.insert(name, preview.clone());
    preview
}

/// A coordinate with at most two decimals and no trailing zeros, so float noise such as
/// 29.000000953674316 reads as 29.
fn short(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

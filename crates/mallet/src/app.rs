//! Top-level editor window: menus, Hammer's four-viewport layout and the status bar. The
//! toolbar, map tabs, tool palette and side panel are in `chrome`.

mod chrome;
mod compile_window;
pub(crate) use compile_window::count;
mod options;
mod properties;
mod texture_browser;
mod texture_tool;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Id, Modifiers, PointerButton, Pos2, Rect, Response, Sense, ViewportCommand};
use glam::{DVec2, DVec3};
use mallet_vmf::geometry::Bounds;

use eframe::egui_wgpu;

use crate::branding;
use crate::canvas::{Canvas, PainterCanvas};
use crate::config::{self, Config};
use crate::editor::{self, Editor, SelectMode, Tool, ViewMode, WindowShot, VIEWPORT_GAP};
use crate::keymap::{Action, KeyMap};
use crate::netgraph::{NetGraph, NetGraphExtra};
use crate::render3d::{CameraStyle, Renderer3D};
use crate::scene::Remap;
use crate::view2d::{Axis2D, Handle, View2D};
use crate::view3d::Camera3D;
use crate::{mcp, tools};

/// Port the MCP server listens on unless `MALLET_MCP_PORT` says otherwise.
const DEFAULT_MCP_PORT: u16 = 7414;
/// How close, in pixels, a 2D click has to be to a brush's outline to pick it.
const PICK_TOLERANCE_PIXELS: f64 = 4.0;
/// How long `send_input` waits for the window to handle its input.
const INPUT_TIMEOUT: Duration = Duration::from_secs(5);

/// Actions checked against the keyboard every frame.
const KEY_ACTIONS: [Action; 23] = [
    Action::NewFile,
    Action::CloseTab,
    Action::NextTab,
    Action::PreviousTab,
    Action::OpenFile,
    Action::SaveFile,
    Action::Undo,
    Action::Redo,
    Action::GridSmaller,
    Action::GridLarger,
    Action::SelectionTool,
    Action::BlockTool,
    Action::EntityTool,
    Action::TextureTool,
    Action::Properties,
    Action::Compile,
    Action::CompileAgain,
    Action::SelectAll,
    Action::ClearSelection,
    Action::DeleteSelection,
    Action::CreateBlock,
    Action::CenterViews2D,
    Action::CenterView3D,
];

/// A left-button drag in progress in one 2D view. `start` is the 2D world point where the
/// button went down; `bounds` is the box being moved or resized as it was at that moment.
#[derive(Clone, Copy, Debug)]
enum Drag {
    /// Moving the selection, or cloning it when Shift is held at release.
    Move { view: usize, start: DVec2, bounds: Bounds },
    /// Resizing the selection with one of its handles.
    Resize {
        view: usize,
        start: DVec2,
        bounds: Bounds,
        handle: Handle,
    },
    /// Drawing a selection rectangle.
    Marquee { view: usize, start: DVec2 },
    /// Drawing a new block, from a grid-snapped corner.
    NewBlock { view: usize, start: DVec2 },
    /// Moving the pending block.
    MoveBlock { view: usize, start: DVec2, bounds: Bounds },
    /// Resizing the pending block with one of its handles.
    ResizeBlock {
        view: usize,
        start: DVec2,
        bounds: Bounds,
        handle: Handle,
    },
}

impl Drag {
    fn view(self) -> usize {
        match self {
            Self::Move { view, .. }
            | Self::Resize { view, .. }
            | Self::Marquee { view, .. }
            | Self::NewBlock { view, .. }
            | Self::MoveBlock { view, .. }
            | Self::ResizeBlock { view, .. } => view,
        }
    }
}

/// Pointer and keyboard state a 2D tool needs for one frame.
struct PointerState {
    position: Option<Pos2>,
    press_origin: Option<Pos2>,
    modifiers: Modifiers,
}

/// The editor window. Map and view state live in the shared [`Editor`].
pub struct MalletApp {
    editor: Arc<Mutex<Editor>>,
    keymap: KeyMap,
    cursor: Option<DVec2>,
    title: String,
    mcp_url: Option<String>,
    drag: Option<Drag>,
    /// Each viewport slot's GPU image as registered with egui.
    slot_textures: [Option<egui::TextureId>; 4],
    /// Where the cursor was when mouselook started, to put it back afterwards.
    look_return: Option<Pos2>,
    /// The tab whose "unsaved changes" question is showing.
    closing: Option<usize>,
    /// Material previews for the side panel.
    texture_previews: chrome::TexturePreviews,
    /// Frame timing overlay in the camera viewport's bottom-right corner.
    net_graph: NetGraph,
    /// The compile window's preset, tab and other state.
    compile: compile_window::CompileWindowState,
    /// The Mallet Options window's section.
    options: options::OptionsState,
    /// Mallet's icon, for the menu bar and the About window; loaded on the first frame.
    logo: Option<egui::TextureHandle>,
    /// Face Edit and texture browser state for the texture tool.
    face_edit: texture_tool::FaceEditState,
    texture_browser: texture_browser::TextureBrowserState,
    /// The Object Properties window's tab, drafts and other state.
    properties: properties::PropertiesState,
}

impl MalletApp {
    /// Creates the window, starts the MCP server and opens `path` if one was given.
    ///
    /// The server listens on port 7414, or on `MALLET_MCP_PORT` when set; a value of
    /// `0` or `off` disables it.
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        // Always dark, like Hammer++, whatever Windows uses or a saved layout says.
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let mut app = Self {
            editor: Arc::new(Mutex::new(Editor::default())),
            keymap: KeyMap::hammer_defaults(),
            cursor: None,
            title: String::new(),
            mcp_url: None,
            drag: None,
            slot_textures: [None; 4],
            look_return: None,
            closing: None,
            texture_previews: chrome::TexturePreviews::new(),
            net_graph: NetGraph::default(),
            compile: compile_window::CompileWindowState::default(),
            logo: None,
            face_edit: texture_tool::FaceEditState::default(),
            texture_browser: texture_browser::TextureBrowserState::default(),
            options: options::OptionsState::default(),
            properties: properties::PropertiesState::default(),
        };
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);
        app.start_mcp(&cc.egui_ctx);
        let mut editor = editor::lock(&app.editor);
        load_game_content(&mut editor);
        open_or_new(&mut editor, path);
        drop(editor);
        app
    }

    fn start_mcp(&mut self, ctx: &egui::Context) {
        let setting = std::env::var("MALLET_MCP_PORT").ok();
        let port = match setting.as_deref().map(str::trim) {
            None | Some("") => DEFAULT_MCP_PORT,
            Some("off") => return,
            Some(value) => match value.parse() {
                Ok(0) => return,
                Ok(port) => port,
                Err(_) => {
                    editor::lock(&self.editor).message = format!("MALLET_MCP_PORT \"{value}\" is not a port number");
                    return;
                }
            },
        };
        let shared = Arc::clone(&self.editor);
        let ctx = ctx.clone();
        let handler = move |name: &str, arguments: &serde_json::Value| {
            if name == "screenshot" && arguments.get("viewport").and_then(serde_json::Value::as_str) == Some("window") {
                return match window_screenshot(&shared, &ctx) {
                    Ok(png) => mcp::ToolResult::png(png),
                    Err(message) => mcp::ToolResult::error(message),
                };
            }
            let result = tools::run(&mut editor::lock(&shared), name, arguments);
            ctx.request_repaint();
            if name == "compile_map" && !result.is_error {
                let wait = arguments
                    .get("wait_seconds")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0);
                if wait > 0.0 {
                    wait_for_compile(&shared, Duration::from_secs_f64(wait.min(3600.0)));
                    let mut editor = editor::lock(&shared);
                    editor.poll_compile();
                    return mcp::ToolResult::json(&tools::compile_status_json(&editor, 40));
                }
            }
            if name == "send_input" && !result.is_error {
                if let Err(message) = wait_for_input(&shared, &ctx) {
                    return mcp::ToolResult::error(message);
                }
            }
            result
        };
        match mcp::start(port, tools::definitions(), handler) {
            Ok(url) => self.mcp_url = Some(url),
            Err(error) => {
                editor::lock(&self.editor).message = format!("MCP server could not start on port {port}: {error}");
            }
        }
    }

    fn handle_input(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let keymap = &self.keymap;
        let triggered = ctx.input(|input| {
            KEY_ACTIONS
                .into_iter()
                .filter(|action| keymap.pressed(input, *action))
                .collect::<Vec<_>>()
        });
        if ctx.wants_keyboard_input() {
            return;
        }
        for action in triggered {
            if action == Action::ClearSelection && editor.camera.mouse_look() {
                continue;
            }
            self.dispatch(action, editor);
        }
        let dropped = ctx.input(|input| input.raw.dropped_files.iter().find_map(|file| file.path.clone()));
        if let Some(path) = dropped {
            let _ = editor.open_in_tab(&path);
        }
    }

    fn update_title(&mut self, ctx: &egui::Context, editor: &Editor) {
        let title = match &editor.document {
            Some(document) => format!("Mallet - {}", document.file_name()),
            None => "Mallet".to_owned(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut chosen = None;
        let mut save_as = false;
        let mut choose_game = false;
        let mut choose_fgds = false;
        let mut show_net_graph = editor.config.view.net_graph;
        let mut draw_distance = editor.config.view.draw_distance;
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                let logo = self.logo.get_or_insert_with(|| branding::logo_texture(ctx)).id();
                ui.add(egui::Image::new((logo, egui::Vec2::splat(16.0))))
                    .on_hover_text(format!("Mallet {}", env!("CARGO_PKG_VERSION")));
                ui.menu_button("File", |ui| {
                    menu_item(ui, &self.keymap, "New", Action::NewFile, &mut chosen);
                    menu_item(ui, &self.keymap, "Open...", Action::OpenFile, &mut chosen);
                    menu_item(ui, &self.keymap, "Save", Action::SaveFile, &mut chosen);
                    menu_item(ui, &self.keymap, "Close Tab", Action::CloseTab, &mut chosen);
                    if ui.button("Save As...").clicked() {
                        save_as = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    menu_item(ui, &self.keymap, "Compile...", Action::Compile, &mut chosen);
                    menu_item(ui, &self.keymap, "Compile Again", Action::CompileAgain, &mut chosen);
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    menu_item(ui, &self.keymap, "Undo", Action::Undo, &mut chosen);
                    menu_item(ui, &self.keymap, "Redo", Action::Redo, &mut chosen);
                    ui.separator();
                    menu_item(ui, &self.keymap, "Delete", Action::DeleteSelection, &mut chosen);
                    menu_item(ui, &self.keymap, "Select All", Action::SelectAll, &mut chosen);
                    menu_item(ui, &self.keymap, "Select None", Action::ClearSelection, &mut chosen);
                    ui.separator();
                    menu_item(ui, &self.keymap, "Properties", Action::Properties, &mut chosen);
                    ui.separator();
                    menu_item(ui, &self.keymap, "Mallet Options...", Action::Options, &mut chosen);
                });
                ui.menu_button("View", |ui| {
                    menu_item(ui, &self.keymap, "Center 2D Views", Action::CenterViews2D, &mut chosen);
                    menu_item(ui, &self.keymap, "Center 3D View", Action::CenterView3D, &mut chosen);
                    ui.separator();
                    if ui.checkbox(&mut show_net_graph, "Net Graph").clicked() {
                        ui.close_menu();
                    }
                    ui.horizontal(|ui| {
                        ui.label("3D draw distance");
                        ui.add(
                            egui::Slider::new(&mut draw_distance, 1024.0..=65536.0)
                                .logarithmic(true)
                                .integer(),
                        );
                    });
                    ui.separator();
                    menu_item(ui, &self.keymap, "Smaller Grid", Action::GridSmaller, &mut chosen);
                    menu_item(ui, &self.keymap, "Larger Grid", Action::GridLarger, &mut chosen);
                });
                ui.menu_button("Tools", |ui| {
                    if ui.button("Game Folder...").clicked() {
                        choose_game = true;
                        ui.close_menu();
                    }
                    if ui.button("Game Data Files (FGD)...").clicked() {
                        choose_fgds = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    menu_item(ui, &self.keymap, "Selection Tool", Action::SelectionTool, &mut chosen);
                    menu_item(ui, &self.keymap, "Block Tool", Action::BlockTool, &mut chosen);
                    menu_item(ui, &self.keymap, "Entity Tool", Action::EntityTool, &mut chosen);
                    menu_item(
                        ui,
                        &self.keymap,
                        "Texture Application",
                        Action::TextureTool,
                        &mut chosen,
                    );
                    menu_item(ui, &self.keymap, "Create Block", Action::CreateBlock, &mut chosen);
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About Mallet").clicked() {
                        editor.about_open = true;
                        ui.close_menu();
                    }
                });
            });
        });
        if let Some(action) = chosen {
            self.dispatch(action, editor);
        }
        if save_as {
            save_as_dialog(editor);
        }
        if choose_game {
            choose_game_dir(editor);
        }
        if choose_fgds {
            choose_fgd_files(editor);
        }
        if show_net_graph != editor.config.view.net_graph || draw_distance != editor.config.view.draw_distance {
            editor.config.view.net_graph = show_net_graph;
            editor.config.view.draw_distance = draw_distance;
            if !ctx.input(|input| input.pointer.any_down()) {
                editor.save_config();
            }
        }
    }

    /// Help > About Mallet: the icon, version, and where things are.
    fn about_window(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        if !editor.about_open {
            return;
        }
        let logo = self.logo.get_or_insert_with(|| branding::logo_texture(ctx)).id();
        let mut open = true;
        egui::Window::new("About Mallet")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.add(egui::Image::new((logo, egui::Vec2::splat(112.0))));
                    ui.add_space(6.0);
                    ui.heading(egui::RichText::new("Mallet").size(26.0).strong());
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.label(egui::RichText::new(env!("CARGO_PKG_DESCRIPTION")).color(egui::Color32::GRAY));
                    ui.add_space(10.0);
                });
                egui::Grid::new("about_details")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Game");
                        ui.label(
                            editor
                                .game_dir()
                                .map_or_else(|| "none".to_owned(), |dir| dir.display().to_string()),
                        );
                        ui.end_row();
                        ui.label("Graphics");
                        ui.label(if editor.gpu_info.is_empty() {
                            "starting"
                        } else {
                            editor.gpu_info.as_str()
                        });
                        ui.end_row();
                        ui.label("MCP server");
                        ui.label(self.mcp_url.as_deref().unwrap_or("off"));
                        ui.end_row();
                        ui.label("Settings");
                        ui.label(config::data_dir().map_or_else(String::new, |dir| dir.display().to_string()));
                        ui.end_row();
                    });
                ui.add_space(6.0);
            });
        if !open {
            editor.about_open = false;
        }
    }

    fn status_bar(&self, ctx: &egui::Context, editor: &Editor) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&editor.message);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("Grid: {}", editor.grid));
                    if let Some(cursor) = self.cursor {
                        ui.label(format!("@{}, {}", cursor.x, cursor.y));
                    }
                    if let Some(summary) = editor.selection_summary() {
                        ui.label(summary);
                    }
                    if editor.document.is_some() {
                        ui.label(format!(
                            "{} solids, {} entities",
                            editor.scene.solid_count, editor.scene.entity_count
                        ));
                    }
                    if let Some(url) = &self.mcp_url {
                        ui.label(format!("MCP: {url}"));
                    }
                    compile_status(ui, editor);
                });
            });
        });
    }

    fn viewports(&mut self, ui: &mut egui::Ui, editor: &mut Editor, frame: &eframe::Frame) {
        let rects = editor::quad_rects(ui.available_rect_before_wrap(), VIEWPORT_GAP);
        editor.set_layout(rects, ui.ctx().pixels_per_point());
        let pointer = ui.input(|input| PointerState {
            position: input.pointer.latest_pos(),
            press_origin: input.pointer.press_origin(),
            modifiers: input.modifiers,
        });
        let space_held = ui.input(|input| input.key_down(egui::Key::Space));
        let mut camera_updated = false;
        let (mut camera_ms, mut views_ms) = (0.0, 0.0);
        self.cursor = None;
        for (slot, rect) in rects.into_iter().enumerate() {
            let started = Instant::now();
            let response = ui.interact(rect, Id::new(("viewport", slot)), Sense::click_and_drag());
            let mode = editor.layout[slot];
            match (mode.camera_style(), mode.view_index()) {
                (Some(style), _) => {
                    self.camera_slot(
                        ui,
                        slot,
                        style,
                        rect,
                        &response,
                        editor,
                        frame,
                        &pointer,
                        &mut camera_updated,
                    );
                    camera_ms += elapsed_ms(started);
                }
                (None, Some(view)) => {
                    self.view_slot(ui, slot, view, rect, &response, editor, frame, &pointer, space_held);
                    views_ms += elapsed_ms(started);
                }
                (None, None) => {}
            }
            viewport_menu(ui, slot, rect, editor);
        }
        editor.timings.camera_ms = camera_ms;
        editor.timings.views_2d_ms = views_ms;
    }

    /// Handles and draws a 3D viewport slot. The camera is shared, so it only takes
    /// input once per frame: in the hovered 3D slot, or the first one during mouselook.
    ///
    /// While mouselook is on the cursor is kept at the viewport's centre, so it never
    /// wanders into another viewport (where the mouse wheel would zoom that view instead),
    /// and it goes back to where it was when mouselook ends.
    #[allow(clippy::too_many_arguments)]
    fn camera_slot(
        &mut self,
        ui: &mut egui::Ui,
        slot: usize,
        style: CameraStyle,
        rect: Rect,
        response: &Response,
        editor: &mut Editor,
        frame: &eframe::Frame,
        pointer: &PointerState,
        camera_updated: &mut bool,
    ) {
        let ctx = ui.ctx().clone();
        let was_looking = editor.camera.mouse_look();
        let mut moving = false;
        let typing = ctx.wants_keyboard_input() && !was_looking;
        if !*camera_updated && !typing && (response.hovered() || was_looking) {
            *camera_updated = true;
            moving = editor
                .camera
                .update(&ctx, response.hovered() || was_looking, &self.keymap);
            let looking = editor.camera.mouse_look();
            if looking && !was_looking {
                self.look_return = pointer.position;
            }
            if looking {
                let center = rect.center();
                if pointer.position.is_none_or(|position| position.distance(center) > 1.0) {
                    ctx.send_viewport_cmd(ViewportCommand::CursorPosition(center));
                }
            } else if was_looking {
                if let Some(position) = self.look_return.take() {
                    ctx.send_viewport_cmd(ViewportCommand::CursorPosition(position));
                }
            }
        }
        let looking = editor.camera.mouse_look();
        let clicked = if looking {
            ui.input(|input| input.pointer.primary_pressed())
        } else {
            response.clicked_by(PointerButton::Primary)
        };
        let aim = if looking { Some(rect.center()) } else { pointer.position };
        if let (true, Some(aim)) = (clicked, aim) {
            let (origin, direction) = editor.camera.ray(rect, aim);
            match editor.tool {
                Tool::Select => {
                    let hit = editor.scene.pick_ray(origin, direction);
                    pick(editor, hit, pointer.modifiers);
                    // Double-clicking opens the properties of what was picked, as in Hammer.
                    if response.double_clicked_by(PointerButton::Primary) && !editor.selection.is_empty() {
                        editor.properties_open = true;
                    }
                }
                Tool::Entity => match editor.scene.surface_hit(origin, direction) {
                    Some(point) => {
                        // Adding zero turns -0 into 0, so the origin is never written as "-0".
                        let classname = editor.entity_class.clone();
                        if let Err(error) = editor.create_entity(&classname, point.round() + DVec3::ZERO, &[]) {
                            editor.message = error;
                        }
                    }
                    None => editor.message = "Click on a brush face to place the entity".to_owned(),
                },
                Tool::Texture => {
                    let hit = editor.scene.pick_face(origin, direction);
                    editor.texture_click(hit, pointer.modifiers);
                }
                Tool::Block => {}
            }
        }
        if editor.tool == Tool::Texture {
            let applied = if looking {
                ui.input(|input| input.pointer.secondary_pressed())
            } else {
                response.clicked_by(PointerButton::Secondary)
            };
            if let (true, Some(aim)) = (applied, aim) {
                let (origin, direction) = editor.camera.ray(rect, aim);
                let hit = editor.scene.pick_face(origin, direction);
                editor.texture_apply_click(hit, pointer.modifiers.alt);
            }
        }
        let active = response.hovered() || moving;
        let painter = ui.painter_at(rect);
        let canvas = &mut PainterCanvas(&painter);
        match frame.wgpu_render_state() {
            Some(render_state) => {
                let texture = self.render_camera(render_state, editor, slot, style);
                let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(texture, rect, full, egui::Color32::WHITE);
                let label = match style {
                    CameraStyle::Textured if editor.materials.is_none() => "camera (no game set)",
                    _ => editor.layout[slot].label(),
                };
                let color = if active {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::GRAY
                };
                canvas.label(rect.left_top() + egui::vec2(6.0, 4.0), label, color);
                Camera3D::draw_crosshair(&painter, rect);
                if editor.config.view.net_graph && style == CameraStyle::Textured {
                    let extra = editor.gpu.as_ref().map(|gpu| NetGraphExtra {
                        stats: gpu.stats,
                        reused: gpu.reused.iter().filter(|reused| **reused).count(),
                        loading: gpu.textures_loading(),
                    });
                    self.net_graph.draw(&painter, rect, &extra.unwrap_or_default());
                }
                if editor.gpu.as_ref().is_some_and(|gpu| gpu.textures_loading() > 0) {
                    ctx.request_repaint_after(Duration::from_millis(50));
                }
            }
            None => editor
                .camera
                .draw(canvas, rect, &editor.scene, active, &editor.display_overlay()),
        }
        if moving {
            ctx.request_repaint();
        }
    }

    /// Handles and draws a 2D viewport slot showing one of the editor's three 2D views.
    ///
    /// During mouselook the 2D views ignore the mouse: the cursor is only pulled back to the
    /// 3D view once per frame, so a quick flick can leave it over a 2D view for a moment, and
    /// a click there would otherwise undo the selection made through the crosshair.
    #[allow(clippy::too_many_arguments)]
    fn view_slot(
        &mut self,
        ui: &mut egui::Ui,
        slot: usize,
        view_index: usize,
        rect: Rect,
        response: &Response,
        editor: &mut Editor,
        frame: &eframe::Frame,
        pointer: &PointerState,
        space_held: bool,
    ) {
        if !editor.camera.mouse_look() {
            editor.views[view_index].handle_input(ui, response, rect);
        }
        let view = editor.views[view_index].clone();
        if !space_held && !editor.camera.mouse_look() {
            self.use_tool_2d(slot, &view, rect, response, editor, pointer);
            if let Some(icon) = self.cursor_icon(slot, &view, rect, response, editor) {
                ui.ctx().set_cursor_icon(icon);
            }
        }
        let painter = ui.painter_at(rect);
        let canvas = &mut PainterCanvas(&painter);
        let overlay = editor.display_overlay();
        match frame.wgpu_render_state() {
            Some(render_state) => {
                let texture = self.render_view(render_state, editor, slot, view_index, rect);
                let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(texture, rect, full, egui::Color32::WHITE);
                view.draw_overlay(canvas, rect, &overlay, response.hovered());
            }
            None => view.draw(canvas, rect, &editor.scene, editor.grid, response.hovered(), &overlay),
        }
        if let Some(position) = response.hover_pos() {
            self.cursor = Some(snap(view.to_world(rect, position), editor.grid));
        }
    }

    /// Renders a 3D slot on the GPU and returns it as an egui texture.
    fn render_camera(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        editor: &mut Editor,
        slot: usize,
        style: CameraStyle,
    ) -> egui::TextureId {
        let overlay = editor.display_overlay();
        let size = editor.slot_pixels(slot);
        let renderer = editor
            .gpu
            .get_or_insert_with(|| Renderer3D::new(&render_state.device, &render_state.queue));
        let (view, recreated) = renderer.render(
            slot,
            style,
            &editor.scene,
            &overlay,
            &editor.camera,
            size,
            editor.materials.as_mut(),
            &editor.models,
        );
        show_texture(render_state, &mut self.slot_textures[slot], view, recreated)
    }

    /// Renders a 2D slot on the GPU and returns it as an egui texture.
    fn render_view(
        &mut self,
        render_state: &egui_wgpu::RenderState,
        editor: &mut Editor,
        slot: usize,
        view_index: usize,
        rect: Rect,
    ) -> egui::TextureId {
        let overlay = editor.display_overlay();
        let pixels = editor.slot_pixels(slot);
        let renderer = editor
            .gpu
            .get_or_insert_with(|| Renderer3D::new(&render_state.device, &render_state.queue));
        let (view, recreated) = renderer.render_2d(
            slot,
            &editor.views[view_index],
            rect.size(),
            pixels,
            &editor.scene,
            &overlay,
            editor.grid,
        );
        show_texture(render_state, &mut self.slot_textures[slot], view, recreated)
    }

    /// Chooses the drag that a left-button press at `origin` starts with the active tool:
    /// resizing from a handle, moving from inside the box, or starting a new marquee or block.
    fn begin_drag(index: usize, view: &View2D, rect: Rect, origin: Pos2, editor: &Editor) -> Drag {
        let start = view.to_world(rect, origin);
        let target = editor.handle_box();
        let handle = target.and_then(|bounds| view.hit_handle(rect, &bounds, origin));
        let inside = target.is_some_and(|bounds| box_contains(view.axis, &bounds, start));
        match (editor.tool, target, handle, inside) {
            (Tool::Select, Some(bounds), Some(handle), _) => Drag::Resize {
                view: index,
                start,
                bounds,
                handle,
            },
            (Tool::Select, Some(bounds), None, true) => Drag::Move {
                view: index,
                start,
                bounds,
            },
            (Tool::Select, ..) => Drag::Marquee { view: index, start },
            (Tool::Block, Some(bounds), Some(handle), _) => Drag::ResizeBlock {
                view: index,
                start,
                bounds,
                handle,
            },
            (Tool::Block, Some(bounds), None, true) => Drag::MoveBlock {
                view: index,
                start,
                bounds,
            },
            (Tool::Entity | Tool::Texture, ..) => Drag::Marquee { view: index, start },
            (Tool::Block, ..) => Drag::NewBlock {
                view: index,
                start: snap(start, editor.grid),
            },
        }
    }

    /// The resize or move cursor while hovering a handle or the inside of the box, or
    /// while dragging one.
    fn cursor_icon(
        &self,
        index: usize,
        view: &View2D,
        rect: Rect,
        response: &Response,
        editor: &Editor,
    ) -> Option<egui::CursorIcon> {
        match self.drag.filter(|drag| drag.view() == index) {
            Some(Drag::Resize { handle, .. } | Drag::ResizeBlock { handle, .. }) => return Some(handle.cursor()),
            Some(Drag::Move { .. } | Drag::MoveBlock { .. }) => return Some(egui::CursorIcon::Move),
            Some(_) => return None,
            None => {}
        }
        let pointer = response.hover_pos()?;
        let bounds = editor.handle_box()?;
        if let Some(handle) = view.hit_handle(rect, &bounds, pointer) {
            return Some(handle.cursor());
        }
        box_contains(view.axis, &bounds, view.to_world(rect, pointer)).then_some(egui::CursorIcon::Move)
    }

    /// Runs the active tool for a left-button click or drag in one 2D view.
    fn use_tool_2d(
        &mut self,
        index: usize,
        view: &View2D,
        rect: Rect,
        response: &Response,
        editor: &mut Editor,
        pointer: &PointerState,
    ) {
        let grid = editor.grid;
        if editor.tool == Tool::Texture {
            // In 2D the texture tool picks whole brushes, as faces overlap in these views.
            if response.clicked_by(PointerButton::Primary) {
                if let Some(position) = pointer.position {
                    let tolerance = PICK_TOLERANCE_PIXELS / view.scale;
                    let hit = editor
                        .scene
                        .pick_2d(view.axis, view.to_world(rect, position), tolerance);
                    editor.texture_click_2d(hit, pointer.modifiers);
                }
            }
            return;
        }
        if editor.tool == Tool::Entity {
            // Hammer's entity tool: pressing or dragging in a 2D view moves the marker
            // for the new entity; Enter creates it.
            let held = response.is_pointer_button_down_on() && response.ctx.input(|input| input.pointer.primary_down());
            if held || response.clicked_by(PointerButton::Primary) {
                if let Some(position) = pointer.position {
                    editor.place_entity_marker(view.axis, view.to_world(rect, position));
                }
            }
            return;
        }
        if response.drag_started_by(PointerButton::Primary) {
            if let Some(origin) = pointer.press_origin {
                self.drag = Some(Self::begin_drag(index, view, rect, origin, editor));
            }
        }

        let Some(drag) = self.drag.filter(|drag| drag.view() == index) else {
            if response.clicked_by(PointerButton::Primary) && editor.tool == Tool::Select {
                if let Some(position) = pointer.position {
                    let tolerance = PICK_TOLERANCE_PIXELS / view.scale;
                    let hit = editor
                        .scene
                        .pick_2d(view.axis, view.to_world(rect, position), tolerance);
                    pick(editor, hit, pointer.modifiers);
                }
                if response.double_clicked_by(PointerButton::Primary) && !editor.selection.is_empty() {
                    editor.properties_open = true;
                }
            }
            return;
        };

        if let Some(position) = pointer.position {
            let world = view.to_world(rect, position);
            match drag {
                Drag::Move { start, bounds, .. } => {
                    let offset = snapped_offset(view.axis, &bounds, world - start, grid);
                    editor.overlay.transform = Some(Remap::translate(bounds, offset));
                }
                Drag::Resize {
                    start, bounds, handle, ..
                } => {
                    let to = handle.resize(view.axis, &bounds, world - start, grid);
                    editor.overlay.transform = Some(Remap { from: bounds, to });
                }
                Drag::Marquee { start, .. } => editor.overlay.marquee = Some((view.axis, start, world)),
                Drag::NewBlock { start, .. } => {
                    editor.overlay.block = editor.block_from_drag(view.axis, start, snap(world, grid));
                }
                Drag::MoveBlock { start, bounds, .. } => {
                    let offset = snapped_offset(view.axis, &bounds, world - start, grid);
                    editor.overlay.block = Some(Remap::translate(bounds, offset).to);
                }
                Drag::ResizeBlock {
                    start, bounds, handle, ..
                } => {
                    editor.overlay.block = Some(handle.resize(view.axis, &bounds, world - start, grid));
                }
            }
        }

        if response.drag_stopped() {
            self.drag = None;
            if let Err(error) = finish_drag(drag, editor, pointer.modifiers) {
                editor.message = error;
            }
        }
    }
}

/// Applies a finished drag: moves, clones or resizes the selection, or selects what the
/// marquee covers. Block drags only change the pending block, which Enter creates.
fn finish_drag(drag: Drag, editor: &mut Editor, modifiers: Modifiers) -> Result<(), String> {
    match drag {
        Drag::Move { .. } => {
            let Some(remap) = editor.overlay.transform.take() else {
                return Ok(());
            };
            let offset = remap.to.min - remap.from.min;
            match (offset == DVec3::ZERO, modifiers.shift) {
                (true, _) => Ok(()),
                (false, true) => editor.clone_selection(offset),
                (false, false) => editor.move_selection(offset),
            }
        }
        Drag::Resize { .. } => match editor.overlay.transform.take() {
            Some(remap) if remap.to != remap.from => editor.resize_selection(remap.to),
            _ => Ok(()),
        },
        Drag::Marquee { .. } => {
            if let Some((axis, a, b)) = editor.overlay.marquee.take() {
                let owners = editor.scene.owners_inside(axis, a, b);
                let mode = if modifiers.ctrl {
                    SelectMode::Add
                } else {
                    SelectMode::Replace
                };
                editor.select(owners, mode);
            }
            Ok(())
        }
        Drag::NewBlock { .. } | Drag::MoveBlock { .. } | Drag::ResizeBlock { .. } => Ok(()),
    }
}

/// The world offset that moves a box by a dragged distance so its minimum corner lands on
/// the grid, the way Hammer snaps moves.
fn snapped_offset(axis: Axis2D, bounds: &Bounds, dragged: DVec2, grid: f64) -> DVec3 {
    let corner = axis.project(bounds.min);
    axis.lift(snap(corner + dragged, grid) - corner)
}

fn box_contains(axis: Axis2D, bounds: &Bounds, point: DVec2) -> bool {
    point.cmpge(axis.project(bounds.min)).all() && point.cmple(axis.project(bounds.max)).all()
}

impl eframe::App for MalletApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let update_started = Instant::now();
        let shared = Arc::clone(&self.editor);
        let mut editor = editor::lock(&shared);
        editor.timings.previous_frame_cpu_ms = frame.info().cpu_usage.map_or(0.0, |seconds| seconds * 1000.0);
        let interval = ctx.input(|input| input.unstable_dt);
        if editor.gpu_info.is_empty() {
            if let Some(render_state) = frame.wgpu_render_state() {
                let info = render_state.adapter.get_info();
                editor.gpu_info = format!("{} ({:?})", info.name, info.backend);
            }
        }
        self.net_graph.record(editor.timings.previous_frame_cpu_ms, interval);
        let [min, average, max] = self.net_graph.interval_stats();
        editor.timings.interval_min_ms = min;
        editor.timings.interval_average_ms = average;
        editor.timings.interval_max_ms = max;
        editor.timings.recent_intervals_ms = self.net_graph.recent_intervals(40);
        self.handle_input(ctx, &mut editor);
        take_window_shot(ctx, &mut editor);
        self.update_title(ctx, &editor);
        self.menu_bar(ctx, &mut editor);
        self.toolbar(ctx, &mut editor);
        self.tab_strip(ctx, &mut editor);
        self.status_bar(ctx, &editor);
        self.tool_palette(ctx, &mut editor);
        self.side_panel(ctx, &mut editor);
        self.close_dialog(ctx, &mut editor);
        self.properties_window(ctx, &mut editor);
        self.compile_window(ctx, &mut editor);
        self.face_edit_window(ctx, &mut editor);
        self.texture_browser_window(ctx, &mut editor);
        self.options_window(ctx, &mut editor);
        self.about_window(ctx, &mut editor);
        self.finish_compile(ctx, &mut editor);
        apply_view_options(&mut editor);
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::from_gray(30)))
            .show(ctx, |ui| self.viewports(ui, &mut editor, frame));
        editor.timings.update_ms = elapsed_ms(update_started);
        editor.frames_drawn += 1;
    }

    /// Feeds one frame of queued synthetic input from the `send_input` tool.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let mut editor = editor::lock(&self.editor);
        if let Some(frame) = editor.input_queue.pop_front() {
            raw_input.modifiers = frame.modifiers;
            raw_input.events.extend(frame.events);
            ctx.request_repaint();
        }
    }
}

/// Copies the viewport settings from Mallet Options onto the camera every frame, so
/// changes show at once.
fn apply_view_options(editor: &mut Editor) {
    let view = &editor.config.view;
    editor.camera.far = view.draw_distance;
    editor.camera.fov = view.fov;
    editor.camera.move_speed = view.camera_speed;
    editor.camera.look_sensitivity = view.look_sensitivity;
}

/// A small progress readout of the running or last compile, in the status bar.
fn compile_status(ui: &mut egui::Ui, editor: &Editor) {
    let Some(job) = &editor.compile_job else { return };
    let job = job.lock();
    let text = match &job.outcome {
        None => {
            let step = job
                .steps
                .iter()
                .find(|step| step.status == crate::compile::StepStatus::Running)
                .map_or("Starting", |step| step.title.as_str());
            ui.add(
                egui::ProgressBar::new(job.progress())
                    .desired_width(90.0)
                    .desired_height(12.0),
            );
            format!("Compiling: {step} {:.0}%", job.progress() * 100.0)
        }
        Some(crate::compile::Outcome::Succeeded) => format!("Compiled {}", job.map),
        Some(crate::compile::Outcome::Failed(_)) => format!("Compile of {} failed", job.map),
        Some(crate::compile::Outcome::Cancelled) => "Compile cancelled".to_owned(),
    };
    ui.label(text);
}

/// Waits, without holding the editor, until the compile ends or the time is up.
fn wait_for_compile(shared: &Mutex<Editor>, limit: Duration) {
    let Some(job) = editor::lock(shared).compile_job.as_ref().map(|job| job.shared()) else {
        return;
    };
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if job.lock().is_ok_and(|state| !state.is_running()) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Asks the window for a screenshot of itself and waits for the PNG.
fn window_screenshot(shared: &Mutex<Editor>, ctx: &egui::Context) -> Result<Vec<u8>, String> {
    editor::lock(shared).window_shot = WindowShot::Requested;
    let deadline = Instant::now() + INPUT_TIMEOUT;
    while Instant::now() < deadline {
        {
            let mut editor = editor::lock(shared);
            if let WindowShot::Done(_) = editor.window_shot {
                let WindowShot::Done(result) = std::mem::take(&mut editor.window_shot) else {
                    unreachable!("matched above");
                };
                return result;
            }
        }
        ctx.request_repaint();
        thread::sleep(Duration::from_millis(5));
    }
    editor::lock(shared).window_shot = WindowShot::Idle;
    Err("the window did not take the screenshot in time; it may be minimized".to_owned())
}

/// Sends a requested window screenshot to the GPU and encodes the image when it arrives.
fn take_window_shot(ctx: &egui::Context, editor: &mut Editor) {
    match editor.window_shot {
        WindowShot::Requested => {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
            editor.window_shot = WindowShot::Waiting;
            ctx.request_repaint();
        }
        WindowShot::Waiting => {
            let image = ctx.input(|input| {
                input.raw.events.iter().find_map(|event| match event {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            match image {
                Some(image) => editor.window_shot = WindowShot::Done(encode_screenshot(&image)),
                None => ctx.request_repaint(),
            }
        }
        _ => {}
    }
}

fn encode_screenshot(image: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let [width, height] = image.size;
    let size = tiny_skia::IntSize::from_wh(width as u32, height as u32).ok_or("the window has no area")?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
    let pixmap = tiny_skia::Pixmap::from_vec(bytes, size).ok_or("the screenshot has the wrong size")?;
    pixmap.encode_png().map_err(|error| error.to_string())
}

/// Waits until the window has handled all queued synthetic input, including the frame
/// that consumed the last of it.
fn wait_for_input(shared: &Mutex<Editor>, ctx: &egui::Context) -> Result<(), String> {
    let deadline = Instant::now() + INPUT_TIMEOUT;
    let mut drained_at = None;
    while Instant::now() < deadline {
        {
            let editor = editor::lock(shared);
            match drained_at {
                None if editor.input_queue.is_empty() => drained_at = Some(editor.frames_drawn),
                Some(frame) if editor.frames_drawn > frame => return Ok(()),
                _ => {}
            }
        }
        ctx.request_repaint();
        thread::sleep(Duration::from_millis(5));
    }
    editor::lock(shared).input_queue.clear();
    Err("the window did not handle the input in time; it may be minimized".to_owned())
}

/// Mounts the configured game folder, or finds an installed Source game the first time
/// and remembers it.
fn load_game_content(editor: &mut Editor) {
    editor.config = Config::load();
    editor.grid = editor.config.view.default_grid;
    let config = editor.config.clone();
    let detected = config.game_dir.is_none();
    let Some(game_dir) = config.game_dir.clone().or_else(config::detect_game_dir) else {
        return;
    };
    if editor.set_game_dir(&game_dir).is_ok() && detected {
        editor.config.game_dir = Some(game_dir);
        editor.save_config();
    }
    if !config.fgds.is_empty() {
        if let Err(error) = editor.set_fgds(&config.fgds) {
            editor.message = format!("Could not read the chosen FGD files: {error}");
        }
    }
}

/// Asks for the FGD files to read entity classes from, reads them and remembers them.
fn choose_fgd_files(editor: &mut Editor) {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Choose the game's FGD files")
        .add_filter("Forge Game Data", &["fgd"]);
    if let Some(bin) = editor
        .game_dir()
        .and_then(Path::parent)
        .map(|parent| parent.join("bin"))
    {
        dialog = dialog.set_directory(bin);
    }
    let Some(paths) = dialog.pick_files() else {
        return;
    };
    if let Err(error) = editor.choose_fgds(paths) {
        editor.message = error;
    }
}

/// Asks for the game folder that holds `gameinfo.txt`, mounts it and remembers it.
fn choose_game_dir(editor: &mut Editor) {
    let Some(folder) = rfd::FileDialog::new()
        .set_title("Choose the game folder that contains gameinfo.txt")
        .pick_folder()
    else {
        return;
    };
    if let Err(error) = editor.choose_game(&folder) {
        editor.message = error;
    }
}

/// Opens the map given on the command line, or starts a new map like Hammer when there is
/// none or it fails to load. A load failure stays in the status bar.
fn open_or_new(editor: &mut Editor, path: Option<PathBuf>) {
    let Some(path) = path else {
        editor.new_map();
        return;
    };
    if editor.open(&path).is_err() {
        let failure = std::mem::take(&mut editor.message);
        editor.new_map();
        editor.message = failure;
    }
}

/// Draws a viewport's label as a button: clicking it opens a menu to switch the slot
/// between the 3D and 2D view types.
fn viewport_menu(ui: &mut egui::Ui, slot: usize, rect: Rect, editor: &mut Editor) {
    let mode = editor.layout[slot];
    let font = egui::FontId::proportional(13.0);
    let width = ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(mode.label().to_owned(), font, egui::Color32::WHITE)
            .size()
            .x
    });
    let label = Rect::from_min_size(rect.left_top() + egui::vec2(3.0, 2.0), egui::vec2(width + 8.0, 19.0));
    let response = ui
        .interact(label, Id::new(("viewport_label", slot)), Sense::click())
        .on_hover_text("Change this viewport's type");
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter_at(rect).rect_stroke(
            label,
            2.0,
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(120)),
            egui::StrokeKind::Inside,
        );
    }
    let popup = Id::new(("viewport_menu", slot));
    if response.clicked() {
        ui.memory_mut(|memory| memory.toggle_popup(popup));
    }
    egui::popup::popup_below_widget(ui, popup, &response, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        ui.set_min_width(140.0);
        for (index, choice) in ViewMode::ALL.into_iter().enumerate() {
            if index == 2 {
                ui.separator();
            }
            if ui.selectable_label(choice == mode, choice.menu_label()).clicked() {
                editor.layout[slot] = choice;
            }
        }
    });
}

/// Registers a GPU image with egui the first time, and refreshes egui's reference when the
/// image was recreated.
fn show_texture(
    render_state: &egui_wgpu::RenderState,
    slot: &mut Option<egui::TextureId>,
    view: &eframe::wgpu::TextureView,
    recreated: bool,
) -> egui::TextureId {
    let mut egui_renderer = render_state.renderer.write();
    let filter = eframe::wgpu::FilterMode::Nearest;
    match *slot {
        Some(id) if !recreated => id,
        Some(id) => {
            egui_renderer.update_egui_texture_from_wgpu_texture(&render_state.device, view, filter, id);
            id
        }
        None => {
            let id = egui_renderer.register_native_texture(&render_state.device, view, filter);
            *slot = Some(id);
            id
        }
    }
}

fn elapsed_ms(started: Instant) -> f32 {
    started.elapsed().as_secs_f32() * 1000.0
}

fn snap(point: DVec2, grid: f64) -> DVec2 {
    (point / grid).round() * grid
}

/// Applies a click: selects what was hit, or with Ctrl toggles it. Clicking empty space
/// without Ctrl clears the selection.
fn pick(editor: &mut Editor, hit: Option<crate::scene::Owner>, modifiers: Modifiers) {
    match (hit, modifiers.ctrl) {
        (Some(owner), true) => editor.select([owner], SelectMode::Toggle),
        (Some(owner), false) => editor.select([owner], SelectMode::Replace),
        (None, true) => {}
        (None, false) => editor.clear_selection(),
    }
}

fn run(action: Action, editor: &mut Editor) {
    match action {
        Action::OpenFile => {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Valve Map File", &["vmf"])
                .pick_file()
            {
                let _ = editor.open_in_tab(&path);
            }
        }
        Action::NewFile => editor.new_tab(),
        Action::SaveFile => {
            if editor.is_untitled() {
                save_as_dialog(editor);
            } else {
                let _ = editor.save(None);
            }
        }
        Action::Undo => {
            let _ = editor.step_history(false);
        }
        Action::Redo => {
            let _ = editor.step_history(true);
        }
        Action::GridSmaller => editor.change_grid(0.5),
        Action::GridLarger => editor.change_grid(2.0),
        Action::SelectionTool => editor.set_tool(Tool::Select),
        Action::BlockTool => editor.set_tool(Tool::Block),
        Action::EntityTool => editor.set_tool(Tool::Entity),
        Action::TextureTool => editor.set_tool(Tool::Texture),
        Action::SelectAll => editor.select_all(),
        Action::ClearSelection => {
            let pending = editor.overlay.block.take().is_some() | editor.overlay.entity.take().is_some();
            if editor.tool == Tool::Texture {
                editor.clear_face_selection();
            } else if !pending {
                editor.clear_selection();
            }
        }
        Action::DeleteSelection => {
            let _ = editor.delete_selection();
        }
        Action::CreateBlock => {
            let result = match editor.tool {
                Tool::Block => editor.commit_block().map(|_| ()),
                Tool::Entity => editor.commit_entity().map(|_| ()),
                Tool::Select | Tool::Texture => Ok(()),
            };
            if let Err(error) = result {
                editor.message = error;
            }
        }
        Action::CenterViews2D => editor.fit_2d(),
        Action::CenterView3D => editor.fit_3d(),
        Action::Properties => editor.properties_open = true,
        Action::Options => editor.options_open = true,
        _ => {}
    }
}

fn save_as_dialog(editor: &mut Editor) {
    let Some(document) = &editor.document else {
        return;
    };
    let picked = rfd::FileDialog::new()
        .add_filter("Valve Map File", &["vmf"])
        .set_file_name(document.file_name())
        .save_file();
    if let Some(path) = picked {
        let _ = editor.save(Some(path));
    }
}

fn menu_item(ui: &mut egui::Ui, keymap: &KeyMap, text: &str, action: Action, chosen: &mut Option<Action>) {
    if ui
        .add(egui::Button::new(text).shortcut_text(keymap.label(action)))
        .clicked()
    {
        *chosen = Some(action);
        ui.close_menu();
    }
}

//! Top-level editor window: menu bar, status bar and Hammer's four-viewport layout.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Id, Sense, ViewportCommand};
use glam::DVec2;

use crate::canvas::PainterCanvas;
use crate::editor::{self, Editor, VIEWPORT_GAP};
use crate::keymap::{Action, KeyMap};
use crate::{mcp, tools};

/// Port the MCP server listens on unless `MALLET_MCP_PORT` says otherwise.
const DEFAULT_MCP_PORT: u16 = 7414;

/// The editor window. Map and view state live in the shared [`Editor`].
pub struct MalletApp {
    editor: Arc<Mutex<Editor>>,
    keymap: KeyMap,
    cursor: Option<DVec2>,
    title: String,
    mcp_url: Option<String>,
}

impl MalletApp {
    /// Creates the window, starts the MCP server and opens `path` if one was given.
    ///
    /// The server listens on port 7414, or on `MALLET_MCP_PORT` when set; a value of
    /// `0` or `off` disables it.
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let mut app = Self {
            editor: Arc::new(Mutex::new(Editor::default())),
            keymap: KeyMap::hammer_defaults(),
            cursor: None,
            title: String::new(),
            mcp_url: None,
        };
        app.start_mcp(&cc.egui_ctx);
        if let Some(path) = path {
            let _ = editor::lock(&app.editor).open(&path);
        }
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
                    editor::lock(&self.editor).message =
                        format!("MALLET_MCP_PORT \"{value}\" is not a port number");
                    return;
                }
            },
        };
        let shared = Arc::clone(&self.editor);
        let ctx = ctx.clone();
        let handler = move |name: &str, arguments: &serde_json::Value| {
            let result = tools::run(&mut editor::lock(&shared), name, arguments);
            ctx.request_repaint();
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
            [
                Action::OpenFile,
                Action::SaveFile,
                Action::Undo,
                Action::Redo,
                Action::GridSmaller,
                Action::GridLarger,
                Action::CenterViews2D,
                Action::CenterView3D,
            ]
            .into_iter()
            .filter(|action| keymap.pressed(input, *action))
            .collect::<Vec<_>>()
        });
        for action in triggered {
            run(action, editor);
        }
        let dropped = ctx.input(|input| input.raw.dropped_files.iter().find_map(|file| file.path.clone()));
        if let Some(path) = dropped {
            let _ = editor.open(&path);
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

    fn menu_bar(&self, ctx: &egui::Context, editor: &mut Editor) {
        let mut chosen = None;
        let mut save_as = false;
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    menu_item(ui, &self.keymap, "Open...", Action::OpenFile, &mut chosen);
                    menu_item(ui, &self.keymap, "Save", Action::SaveFile, &mut chosen);
                    if ui.button("Save As...").clicked() {
                        save_as = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    menu_item(ui, &self.keymap, "Undo", Action::Undo, &mut chosen);
                    menu_item(ui, &self.keymap, "Redo", Action::Redo, &mut chosen);
                });
                ui.menu_button("View", |ui| {
                    menu_item(ui, &self.keymap, "Center 2D Views", Action::CenterViews2D, &mut chosen);
                    menu_item(ui, &self.keymap, "Center 3D View", Action::CenterView3D, &mut chosen);
                    ui.separator();
                    menu_item(ui, &self.keymap, "Smaller Grid", Action::GridSmaller, &mut chosen);
                    menu_item(ui, &self.keymap, "Larger Grid", Action::GridLarger, &mut chosen);
                });
            });
        });
        if let Some(action) = chosen {
            run(action, editor);
        }
        if save_as {
            save_as_dialog(editor);
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
                    if editor.document.is_some() {
                        ui.label(format!(
                            "{} solids, {} entities",
                            editor.scene.solid_count, editor.scene.entity_count
                        ));
                    }
                    if let Some(url) = &self.mcp_url {
                        ui.label(format!("MCP: {url}"));
                    }
                });
            });
        });
    }

    fn viewports(&mut self, ui: &mut egui::Ui, editor: &mut Editor) {
        let rects = editor::quad_rects(ui.available_rect_before_wrap(), VIEWPORT_GAP);
        editor.set_layout(rects);

        let ctx = ui.ctx().clone();
        let response = ui.interact(rects[0], Id::new("viewport_3d"), Sense::click_and_drag());
        let moving = editor.camera.update(&ctx, response.hovered(), &self.keymap);
        let active = response.hovered() || moving;
        editor
            .camera
            .draw(&mut PainterCanvas(&ui.painter_at(rects[0])), rects[0], &editor.scene, active);
        if moving {
            ctx.request_repaint();
        }

        self.cursor = None;
        let grid = editor.grid;
        for (index, view) in editor.views.iter_mut().enumerate() {
            let rect = rects[index + 1];
            let response = ui.interact(rect, Id::new(("viewport_2d", index)), Sense::click_and_drag());
            view.handle_input(ui, &response, rect);
            let canvas = &mut PainterCanvas(&ui.painter_at(rect));
            view.draw(canvas, rect, &editor.scene, grid, response.hovered());
            if let Some(pointer) = response.hover_pos() {
                let world = view.to_world(rect, pointer);
                self.cursor = Some((world / grid).round() * grid);
            }
        }
    }
}

impl eframe::App for MalletApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let shared = Arc::clone(&self.editor);
        let mut editor = editor::lock(&shared);
        self.handle_input(ctx, &mut editor);
        self.update_title(ctx, &editor);
        self.menu_bar(ctx, &mut editor);
        self.status_bar(ctx, &editor);
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::from_gray(30)))
            .show(ctx, |ui| self.viewports(ui, &mut editor));
    }
}

fn run(action: Action, editor: &mut Editor) {
    match action {
        Action::OpenFile => {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Valve Map File", &["vmf"])
                .pick_file()
            {
                let _ = editor.open(&path);
            }
        }
        Action::SaveFile => {
            let _ = editor.save(None);
        }
        Action::Undo => {
            let _ = editor.step_history(false);
        }
        Action::Redo => {
            let _ = editor.step_history(true);
        }
        Action::GridSmaller => editor.change_grid(0.5),
        Action::GridLarger => editor.change_grid(2.0),
        Action::CenterViews2D => editor.fit_2d(),
        Action::CenterView3D => editor.fit_3d(),
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

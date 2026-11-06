//! Mallet Options (Edit > Mallet Options...): every setting in one window, grouped into
//! sections on the left. Changes apply at once and are saved to `config.json`.

use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText};
use egui_phosphor::regular as icon;

use super::compile_window::{open_folder, path_field};
use super::MalletApp;
use crate::compile::default_presets;
use crate::config::{self, ToolKind};
use crate::editor::Editor;
use crate::keymap::KeyMap;

const GOOD: Color32 = Color32::from_rgb(90, 190, 110);
const ERROR: Color32 = Color32::from_rgb(235, 90, 80);
const MUTED: Color32 = Color32::from_gray(140);
const SECTION_WIDTH: f32 = 170.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Section {
    #[default]
    Game,
    Tools,
    Compiling,
    Viewports,
    Keyboard,
    Agents,
}

impl Section {
    const ALL: [Section; 6] = [
        Section::Game,
        Section::Tools,
        Section::Compiling,
        Section::Viewports,
        Section::Keyboard,
        Section::Agents,
    ];

    fn label(self) -> String {
        let (glyph, name) = match self {
            Section::Game => (icon::GAME_CONTROLLER, "Game"),
            Section::Tools => (icon::WRENCH, "Compile tools"),
            Section::Compiling => (icon::HAMMER, "Compiling"),
            Section::Viewports => (icon::MONITOR, "Viewports"),
            Section::Keyboard => (icon::KEYBOARD, "Keyboard"),
            Section::Agents => (icon::ROBOT, "AI agents"),
        };
        format!("{glyph}  {name}")
    }

    pub(super) fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "game" => Section::Game,
            "tools" => Section::Tools,
            "compiling" => Section::Compiling,
            "viewports" => Section::Viewports,
            "keyboard" => Section::Keyboard,
            "agents" => Section::Agents,
            _ => return None,
        })
    }
}

#[derive(Default)]
pub(super) struct OptionsState {
    section: Section,
    /// Settings changed since they were last written; saved once the mouse is released.
    dirty: bool,
}

/// Something to do once the window is drawn.
enum Command {
    ChooseGame(PathBuf),
    DetectGame,
    ChooseFgds(Vec<PathBuf>),
    /// Mount the game folder again, after the mounted games changed.
    Remount,
}

impl MalletApp {
    pub(super) fn options_window(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        if let Some(section) = editor.options_section.take() {
            if let Some(section) = Section::from_key(&section) {
                self.options.section = section;
            }
        }
        if !editor.options_open {
            return;
        }
        let state = &mut self.options;
        let keymap = &self.keymap;
        let mcp_url = self.mcp_url.clone();
        let mut open = true;
        let mut commands = Vec::new();
        egui::Window::new(format!("{} Mallet Options", icon::GEAR))
            .id(egui::Id::new("mallet_options"))
            .open(&mut open)
            .default_size([780.0, 520.0])
            .min_width(620.0)
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                let width = ui.available_width();
                let height = ui.available_height().max(380.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(SECTION_WIDTH, height),
                        egui::Layout::top_down_justified(egui::Align::Min),
                        |ui| {
                            ui.set_width(SECTION_WIDTH);
                            for section in Section::ALL {
                                ui.selectable_value(&mut state.section, section, section.label());
                            }
                        },
                    );
                    ui.separator();
                    let content = (width - SECTION_WIDTH - 30.0).max(380.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(content, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(content);
                            egui::ScrollArea::vertical()
                                .id_salt("options_section")
                                .auto_shrink([false, false])
                                .show(ui, |ui| match state.section {
                                    Section::Game => game_section(ui, editor, &mut commands),
                                    Section::Tools => tools_section(ui, editor, content, &mut state.dirty),
                                    Section::Compiling => compiling_section(ui, editor, &mut state.dirty),
                                    Section::Viewports => viewports_section(ui, editor, &mut state.dirty),
                                    Section::Keyboard => keyboard_section(ui, keymap),
                                    Section::Agents => agents_section(ui, mcp_url.as_deref()),
                                });
                        },
                    );
                });
            });
        if !open {
            editor.options_open = false;
        }
        for command in commands {
            let result = match command {
                Command::ChooseGame(folder) => editor.choose_game(&folder).map(|_| ()),
                Command::DetectGame => match config::detect_game_dir() {
                    Some(folder) => editor.choose_game(&folder).map(|_| ()),
                    None => Err("No known Source game was found in the Steam libraries".to_owned()),
                },
                Command::ChooseFgds(paths) => editor.choose_fgds(paths).map(|_| ()),
                Command::Remount => match editor.game_dir().map(PathBuf::from) {
                    Some(folder) => {
                        let fgds = editor.config.fgds.clone();
                        editor.save_config();
                        editor.set_game_dir(&folder).map(|_| ()).and_then(|()| {
                            if fgds.is_empty() {
                                Ok(())
                            } else {
                                editor.set_fgds(&fgds).map(|_| ())
                            }
                        })
                    }
                    None => Ok(()),
                },
            };
            if let Err(error) = result {
                editor.message = error;
            }
        }
        let busy = ctx.input(|input| input.pointer.any_down()) || ctx.wants_keyboard_input();
        if self.options.dirty && (!busy || !editor.options_open) {
            editor.save_config();
            self.options.dirty = false;
        }
    }
}

fn heading(ui: &mut egui::Ui, text: &str, note: &str) {
    ui.heading(text);
    if !note.is_empty() {
        ui.label(RichText::new(note).color(MUTED));
    }
    ui.add_space(6.0);
}

fn game_section(ui: &mut egui::Ui, editor: &mut Editor, commands: &mut Vec<Command>) {
    heading(
        ui,
        "Game",
        "The game whose textures, models and entity classes Mallet uses, and that maps compile for.",
    );
    ui.label(RichText::new("Game folder").strong());
    ui.horizontal(|ui| {
        let folder = editor.game_dir().map(|dir| dir.display().to_string());
        match &folder {
            Some(folder) => {
                ui.label(RichText::new(icon::CHECK_CIRCLE).color(GOOD));
                ui.label(RichText::new(folder).monospace());
            }
            None => {
                ui.label(RichText::new(format!("{} No game folder", icon::X_CIRCLE)).color(ERROR));
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button(format!("{} Choose...", icon::FOLDER_OPEN)).clicked() {
            if let Some(folder) = rfd::FileDialog::new()
                .set_title("Choose the game folder that contains gameinfo.txt")
                .pick_folder()
            {
                commands.push(Command::ChooseGame(folder));
            }
        }
        if ui
            .button(format!("{} Find an installed game", icon::MAGNIFYING_GLASS))
            .clicked()
        {
            commands.push(Command::DetectGame);
        }
        if let Some(folder) = editor.game_dir() {
            if ui.button(format!("{} Open", icon::ARROW_SQUARE_OUT)).clicked() {
                open_folder(folder);
            }
        }
    });
    if let Some(materials) = &editor.materials {
        ui.label(
            RichText::new(format!(
                "{} search paths mounted",
                materials.filesystem().mounts().len()
            ))
            .small()
            .color(MUTED),
        );
    }

    ui.add_space(12.0);
    let games = editor.game_dir().map(mallet_content::mounted_games).unwrap_or_default();
    if !games.is_empty() {
        ui.add_space(12.0);
        ui.label(RichText::new("Mounted games").strong());
        ui.label(
            RichText::new("Other games Garry's Mod mounts through cfg/mount.cfg, so their textures and models load.")
                .small()
                .color(MUTED),
        );
        let mut changed = false;
        egui::Grid::new("mounted_games")
            .num_columns(2)
            .spacing([12.0, 2.0])
            .show(ui, |ui| {
                for (name, folder) in &games {
                    let mut on = !editor
                        .config
                        .unmounted_games
                        .iter()
                        .any(|skip| skip.eq_ignore_ascii_case(name));
                    let exists = folder.is_dir();
                    let label = if exists {
                        RichText::new(name)
                    } else {
                        RichText::new(name).color(MUTED)
                    };
                    if ui.add_enabled(exists, egui::Checkbox::new(&mut on, label)).changed() {
                        editor
                            .config
                            .unmounted_games
                            .retain(|skip| !skip.eq_ignore_ascii_case(name));
                        if !on {
                            editor.config.unmounted_games.push(name.clone());
                        }
                        changed = true;
                    }
                    let note = if exists {
                        folder.display().to_string()
                    } else {
                        format!("{} (not installed)", folder.display())
                    };
                    ui.label(RichText::new(note).small().monospace().color(MUTED));
                    ui.end_row();
                }
            });
        if changed {
            commands.push(Command::Remount);
        }
    }

    ui.add_space(12.0);
    ui.label(RichText::new("Entity classes (FGD)").strong());
    match editor.game_data.as_deref() {
        Some(data) => {
            ui.label(RichText::new(format!("{} classes from:", data.len())).color(MUTED));
            for file in &data.files {
                ui.label(RichText::new(file.display().to_string()).small().monospace());
            }
        }
        None => {
            ui.label(RichText::new("No FGD is loaded, so entities show plain keys.").color(MUTED));
        }
    }
    ui.horizontal(|ui| {
        if ui.button(format!("{} Choose FGD files...", icon::FILE_PLUS)).clicked() {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Choose the game's FGD files")
                .add_filter("Forge Game Data", &["fgd"]);
            if let Some(bin) = editor
                .game_dir()
                .and_then(|dir| dir.parent())
                .map(|root| root.join("bin"))
            {
                dialog = dialog.set_directory(bin);
            }
            if let Some(paths) = dialog.pick_files() {
                commands.push(Command::ChooseFgds(paths));
            }
        }
        let custom = !editor.config.fgds.is_empty();
        if ui
            .add_enabled(
                custom,
                egui::Button::new(format!("{} Use the game's own FGD", icon::ARROW_COUNTER_CLOCKWISE)),
            )
            .clicked()
        {
            commands.push(Command::ChooseFgds(Vec::new()));
        }
    });
}

fn tools_section(ui: &mut egui::Ui, editor: &mut Editor, width: f32, dirty: &mut bool) {
    heading(
        ui,
        "Compile tools",
        "Leave a field empty to let Mallet find the program next to the game. The menu lists every copy it found, including 64-bit and Hammer++ builds.",
    );
    let game_dir = editor.game_dir().map(PathBuf::from);
    for tool in ToolKind::ALL {
        let candidates = tool.candidates(game_dir.as_deref());
        let chosen = editor.config.tools.get(tool).cloned();
        let resolved = chosen.clone().or_else(|| candidates.first().cloned());
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(width - 20.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(tool.label()).strong());
                match &resolved {
                    Some(path) if path.is_file() => {
                        ui.label(RichText::new(icon::CHECK_CIRCLE).color(GOOD))
                            .on_hover_text(path.display().to_string());
                        if chosen.is_none() {
                            ui.label(RichText::new("found automatically").small().color(MUTED));
                        }
                    }
                    Some(path) => {
                        ui.label(
                            RichText::new(format!("{} {} does not exist", icon::X_CIRCLE, path.display())).color(ERROR),
                        );
                    }
                    None => {
                        ui.label(RichText::new(format!("{} not found", icon::X_CIRCLE)).color(ERROR));
                    }
                }
            });
            ui.label(RichText::new(tool.description()).small().color(MUTED));
            ui.horizontal(|ui| {
                let mut path = chosen.clone();
                let hint_width = (width - 150.0).max(200.0);
                if path_field(ui, &mut path, hint_width, "exe") {
                    editor.config.tools.set(tool, path);
                    *dirty = true;
                }
                ui.add_enabled_ui(!candidates.is_empty(), |ui| {
                    ui.menu_button(icon::CARET_DOWN, |ui| {
                        for candidate in &candidates {
                            if ui.button(candidate.display().to_string()).clicked() {
                                editor.config.tools.set(tool, Some(candidate.clone()));
                                *dirty = true;
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        if ui.button("Find automatically").clicked() {
                            editor.config.tools.set(tool, None);
                            *dirty = true;
                            ui.close_menu();
                        }
                    })
                    .response
                    .on_hover_text("Programs found on this computer");
                });
            });
            if chosen.is_none() {
                if let Some(found) = &resolved {
                    ui.label(
                        RichText::new(format!("Using {}", found.display()))
                            .small()
                            .monospace()
                            .color(MUTED),
                    );
                }
            }
        });
        ui.add_space(4.0);
    }
}

fn compiling_section(ui: &mut egui::Ui, editor: &mut Editor, dirty: &mut bool) {
    heading(
        ui,
        "Compiling",
        "How compiles run. Presets and their steps are edited in the compile window (F9).",
    );
    let options = &mut editor.config.compile;
    let mut check = |ui: &mut egui::Ui, value: &mut bool, text: &str, hint: &str| {
        if ui.checkbox(value, text).on_hover_text(hint).changed() {
            *dirty = true;
        }
    };
    check(
        ui,
        &mut options.save_before,
        "Save the map before compiling",
        "Compiles always see the latest changes.",
    );
    check(
        ui,
        &mut options.low_priority,
        "Run compilers at low priority",
        "Keeps the editor and the rest of the computer responsive while compiling.",
    );
    check(ui, &mut options.notify, "Flash the taskbar when a compile ends", "");
    check(
        ui,
        &mut options.load_pointfile,
        "Show the leak line when the map leaks",
        "Draws VBSP's pointfile in the views and moves the 3D camera to the leak.",
    );
    check(
        ui,
        &mut options.keep_logs,
        "Keep a log of every compile",
        "Written to %APPDATA%\\Mallet\\logs.",
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui
            .button(format!("{} Open the compile logs", icon::FOLDER_OPEN))
            .clicked()
        {
            if let Some(folder) = config::data_dir().map(|dir| dir.join("logs")) {
                open_folder(&folder);
            }
        }
        if ui
            .button(format!("{} Restore the default presets", icon::ARROWS_CLOCKWISE))
            .on_hover_text("Puts back Fast, Normal, Final, Entities only and Cubemaps. Your own presets stay.")
            .clicked()
        {
            for preset in default_presets() {
                match editor
                    .compile_presets
                    .iter_mut()
                    .find(|existing| existing.name == preset.name)
                {
                    Some(existing) => *existing = preset,
                    None => editor.compile_presets.push(preset),
                }
            }
            editor.save_compile_presets();
        }
    });
}

fn viewports_section(ui: &mut egui::Ui, editor: &mut Editor, dirty: &mut bool) {
    heading(ui, "Viewports", "How the views look and how the 3D camera moves.");
    let view = &mut editor.config.view;
    ui.spacing_mut().slider_width = 260.0;
    egui::Grid::new("viewport_options")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Net graph");
            *dirty |= ui
                .checkbox(&mut view.net_graph, "Show frame timings in the 3D view")
                .changed();
            ui.end_row();
            ui.label("Draw distance");
            *dirty |= ui
                .add(
                    egui::Slider::new(&mut view.draw_distance, 1024.0..=65536.0)
                        .logarithmic(true)
                        .integer()
                        .suffix(" units"),
                )
                .changed();
            ui.end_row();
            ui.label("Field of view");
            *dirty |= ui
                .add(egui::Slider::new(&mut view.fov, 50.0..=120.0).integer().suffix("°"))
                .changed();
            ui.end_row();
            ui.label("Camera speed");
            *dirty |= ui
                .add(
                    egui::Slider::new(&mut view.camera_speed, 100.0..=8000.0)
                        .logarithmic(true)
                        .integer()
                        .suffix(" units/s"),
                )
                .changed();
            ui.end_row();
            ui.label("Mouselook sensitivity");
            *dirty |= ui
                .add(egui::Slider::new(&mut view.look_sensitivity, 0.02..=1.0).logarithmic(true))
                .changed();
            ui.end_row();
            ui.label("Grid for new maps");
            egui::ComboBox::from_id_salt("default_grid")
                .selected_text(format!("{}", view.default_grid))
                .show_ui(ui, |ui| {
                    for size in [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0] {
                        *dirty |= ui
                            .selectable_value(&mut view.default_grid, size, format!("{size}"))
                            .changed();
                    }
                });
            ui.end_row();
        });
    ui.add_space(8.0);
    if ui
        .button(format!("{} Reset viewport options", icon::ARROW_COUNTER_CLOCKWISE))
        .clicked()
    {
        *view = config::ViewOptions::default();
        *dirty = true;
    }
}

fn keyboard_section(ui: &mut egui::Ui, keymap: &KeyMap) {
    heading(ui, "Keyboard", "Hammer's default shortcuts. Rebinding comes later.");
    egui::Grid::new("key_bindings")
        .num_columns(2)
        .striped(true)
        .spacing([24.0, 4.0])
        .show(ui, |ui| {
            for (action, chord) in keymap.bindings() {
                ui.label(action_name(&format!("{action:?}")));
                ui.label(RichText::new(chord.label()).monospace());
                ui.end_row();
            }
        });
}

fn agents_section(ui: &mut egui::Ui, mcp_url: Option<&str>) {
    heading(
        ui,
        "AI agents",
        "Mallet runs an MCP server so agents such as Claude Code can inspect and edit maps, compile them and take screenshots.",
    );
    match mcp_url {
        Some(url) => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::CHECK_CIRCLE).color(GOOD));
                ui.label(RichText::new(url).monospace());
                if ui.small_button(format!("{} Copy", icon::CLIPBOARD_TEXT)).clicked() {
                    ui.ctx().copy_text(url.to_owned());
                }
            });
        }
        None => {
            ui.label(RichText::new(format!("{} The MCP server is off.", icon::X_CIRCLE)).color(ERROR));
        }
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new(
            "Set MALLET_MCP_PORT before starting Mallet to use another port, or to 0 to turn the server off.",
        )
        .color(MUTED),
    );
    let config = r#"{ "mcpServers": { "mallet": { "type": "http", "url": "http://127.0.0.1:7414/mcp" } } }"#;
    ui.add_space(6.0);
    ui.label("Client configuration (.mcp.json):");
    ui.label(RichText::new(config).monospace().small());
    if ui.small_button(format!("{} Copy", icon::CLIPBOARD_TEXT)).clicked() {
        ui.ctx().copy_text(config.to_owned());
    }
}

/// `CenterViews2D` as "Center views 2D".
fn action_name(debug: &str) -> String {
    let mut name = String::new();
    for (index, c) in debug.chars().enumerate() {
        if c.is_ascii_uppercase() && index > 0 {
            name.push(' ');
            name.push(c.to_ascii_lowercase());
        } else {
            name.push(c);
        }
    }
    name
}

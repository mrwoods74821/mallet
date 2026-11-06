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
use crate::shell;
use crate::view3d::MAX_LOOK_FRAMES;

const GOOD: Color32 = Color32::from_rgb(90, 190, 110);
const ERROR: Color32 = Color32::from_rgb(235, 90, 80);
const WARNING: Color32 = Color32::from_rgb(230, 180, 70);
const MUTED: Color32 = Color32::from_gray(140);
const SECTION_WIDTH: f32 = 170.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Section {
    #[default]
    Game,
    Tools,
    Compiling,
    Viewports,
    Colors,
    Editing,
    Files,
    TextureBrowser,
    Interface,
    Keyboard,
    Agents,
}

impl Section {
    const ALL: [Section; 11] = [
        Section::Game,
        Section::Tools,
        Section::Compiling,
        Section::Viewports,
        Section::Colors,
        Section::Editing,
        Section::Files,
        Section::TextureBrowser,
        Section::Interface,
        Section::Keyboard,
        Section::Agents,
    ];

    fn label(self) -> String {
        let (glyph, name) = match self {
            Section::Game => (icon::GAME_CONTROLLER, "Game"),
            Section::Tools => (icon::WRENCH, "Compile tools"),
            Section::Compiling => (icon::HAMMER, "Compiling"),
            Section::Viewports => (icon::MONITOR, "Viewports"),
            Section::Colors => (icon::PALETTE, "Colours"),
            Section::Editing => (icon::CUBE, "Editing"),
            Section::Files => (icon::FILES, "Files"),
            Section::TextureBrowser => (icon::IMAGES, "Texture browser"),
            Section::Interface => (icon::APP_WINDOW, "Interface"),
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
            "colors" => Section::Colors,
            "editing" => Section::Editing,
            "files" => Section::Files,
            "texture_browser" => Section::TextureBrowser,
            "interface" => Section::Interface,
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
                                    Section::Game => game_section(ui, editor, &mut commands, &mut state.dirty),
                                    Section::Tools => tools_section(ui, editor, content, &mut state.dirty),
                                    Section::Compiling => compiling_section(ui, editor, &mut state.dirty),
                                    Section::Viewports => viewports_section(ui, editor, &mut state.dirty),
                                    Section::Colors => colors_section(ui, editor, &mut state.dirty),
                                    Section::Editing => editing_section(ui, editor, &mut state.dirty),
                                    Section::Files => files_section(ui, editor, &mut state.dirty),
                                    Section::TextureBrowser => texture_browser_section(ui, editor, &mut state.dirty),
                                    Section::Interface => interface_section(ui, editor, &mut state.dirty),
                                    Section::Keyboard => keyboard_section(ui, keymap),
                                    Section::Agents => agents_section(ui, editor, mcp_url.as_deref(), &mut state.dirty),
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
                Command::DetectGame => match config::detect_game_dir(editor.config.steam_dir.as_deref()) {
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

fn game_section(ui: &mut egui::Ui, editor: &mut Editor, commands: &mut Vec<Command>, dirty: &mut bool) {
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
            .on_hover_text("Looks in the Steam folder below and every Steam library it lists.")
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
    ui.add_space(8.0);
    ui.label(RichText::new("Steam folder").strong());
    ui.label(
        RichText::new("Where Steam is installed, for finding an installed game. Leave it empty for the usual place.")
            .small()
            .color(MUTED),
    );
    *dirty |= folder_field(ui, &mut editor.config.steam_dir, config::DEFAULT_STEAM_DIR);
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
    egui::Grid::new("compile_options")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Compile folder for .mallet maps").on_hover_text(
                "The compilers read VMF only, so a .mallet map is compiled from a VMF copy in this folder beside it. \
                 The BSP and logs end up there too.",
            );
            *dirty |= ui
                .add_sized(
                    [220.0, ui.spacing().interact_size.y],
                    egui::TextEdit::singleline(&mut options.compile_folder).hint_text(crate::compile::COMPILE_FOLDER),
                )
                .changed();
            ui.end_row();
            ui.label("Output lines kept").on_hover_text(
                "The compile window drops the oldest lines past this, so a long compile cannot use up memory.",
            );
            *dirty |= ui
                .add(
                    egui::DragValue::new(&mut options.output_lines)
                        .range(1000..=1_000_000)
                        .speed(500.0),
                )
                .changed();
            ui.end_row();
        });
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
            ui.label("Camera acceleration")
                .on_hover_text("How quickly the camera speeds up and stops.");
            *dirty |= ui
                .add(
                    egui::Slider::new(&mut view.camera_acceleration, 1.0..=60.0)
                        .logarithmic(true)
                        .suffix(" /s"),
                )
                .changed();
            ui.end_row();
            ui.label("Arrow key turning");
            *dirty |= ui
                .add(
                    egui::Slider::new(&mut view.turn_speed, 10.0..=360.0)
                        .integer()
                        .suffix("°/s"),
                )
                .changed();
            ui.end_row();
            ui.label("Mouse wheel in 3D");
            *dirty |= ui
                .add(
                    egui::Slider::new(&mut view.wheel_step, 8.0..=1024.0)
                        .logarithmic(true)
                        .integer()
                        .suffix(" units per notch"),
                )
                .changed();
            ui.end_row();
            ui.label("Mouselook sensitivity");
            *dirty |= ui
                .add(egui::Slider::new(&mut view.look_sensitivity, 0.02..=1.0).logarithmic(true))
                .changed();
            ui.end_row();
            ui.label("Mouselook smoothing")
                .on_hover_text("Averages mouse movement over this many frames. 1 turns smoothing off.");
            *dirty |= ui
                .add(egui::Slider::new(&mut view.look_smoothing, 1..=MAX_LOOK_FRAMES).suffix(" frames"))
                .changed();
            ui.end_row();
            ui.label("2D zoom per wheel notch");
            let mut percent = ((view.zoom_step - 1.0) * 100.0).round();
            let slider = egui::Slider::new(&mut percent, 5.0..=100.0).integer().suffix("%");
            if ui.add(slider).changed() {
                view.zoom_step = 1.0 + percent / 100.0;
                *dirty = true;
            }
            ui.end_row();
            ui.label("Click tolerance")
                .on_hover_text("How close a click in a 2D view must be to a brush's outline to select it.");
            *dirty |= ui
                .add(egui::Slider::new(&mut view.pick_tolerance, 1.0..=16.0).suffix(" px"))
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
            ui.label("Hide grid lines closer than")
                .on_hover_text("Closer lines are thinned out by doubling their spacing.");
            *dirty |= ui
                .add(
                    egui::Slider::new(&mut view.grid_min_pixels, 2.0..=32.0)
                        .integer()
                        .suffix(" px"),
                )
                .changed();
            ui.end_row();
            ui.label("Highlight grid lines every");
            *dirty |= spacing_choice(
                ui,
                "grid_highlight",
                &mut view.grid_highlight,
                &[32.0, 64.0, 128.0, 256.0, 512.0],
            );
            ui.end_row();
            ui.label("Major grid lines every");
            *dirty |= spacing_choice(
                ui,
                "grid_major",
                &mut view.grid_major,
                &[512.0, 1024.0, 2048.0, 4096.0, 8192.0],
            );
            ui.end_row();
        });
    ui.add_space(8.0);
    if ui
        .button(format!("{} Reset viewport options", icon::ARROW_COUNTER_CLOCKWISE))
        .clicked()
    {
        *view = config::ViewOptions {
            texture_scroll: view.texture_scroll,
            ..config::ViewOptions::default()
        };
        *dirty = true;
    }
}

/// A menu of grid spacings in units, with an entry for none. Returns whether it changed.
fn spacing_choice(ui: &mut egui::Ui, id: &str, value: &mut f64, choices: &[f64]) -> bool {
    let label = |units: f64| {
        if units > 0.0 {
            format!("{units} units")
        } else {
            "Never".to_owned()
        }
    };
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .selected_text(label(*value))
        .show_ui(ui, |ui| {
            for units in choices.iter().copied().chain([0.0]) {
                changed |= ui.selectable_value(value, units, label(units)).changed();
            }
        });
    changed
}

fn colors_section(ui: &mut egui::Ui, editor: &mut Editor, dirty: &mut bool) {
    heading(
        ui,
        "Colors",
        "The colours the viewports are drawn with. The defaults are Hammer's.",
    );
    let colors = &mut editor.config.colors;
    let rows: [(&str, &mut config::Rgb); 9] = [
        ("Background", &mut colors.background),
        ("Grid", &mut colors.grid),
        ("Highlighted grid lines", &mut colors.grid_highlight),
        ("Major grid lines", &mut colors.grid_major),
        ("Axis lines", &mut colors.axis),
        ("Selection", &mut colors.selection),
        ("Block tool and marquee", &mut colors.tool),
        ("Selection handles", &mut colors.handles),
        ("Faces picked for texturing", &mut colors.selected_faces),
    ];
    egui::Grid::new("color_options")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            for (name, color) in rows {
                ui.label(name);
                *dirty |= ui.color_edit_button_srgb(color).changed();
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    if ui
        .button(format!("{} Use Hammer's colours", icon::ARROW_COUNTER_CLOCKWISE))
        .clicked()
    {
        *colors = config::ColorOptions::default();
        *dirty = true;
    }
}

fn editing_section(ui: &mut egui::Ui, editor: &mut Editor, dirty: &mut bool) {
    heading(
        ui,
        "Editing",
        "What Mallet starts with each time it opens. The texture and entity can be changed while you work.",
    );
    let defaults = config::EditingOptions::default();
    let editing = &mut editor.config.editing;
    let mut use_now = None;
    egui::Grid::new("editing_options")
        .num_columns(3)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Texture for new brushes");
            *dirty |= ui
                .add_sized(
                    [260.0, ui.spacing().interact_size.y],
                    egui::TextEdit::singleline(&mut editing.default_material).hint_text(&defaults.default_material),
                )
                .changed();
            if ui
                .button("Use the current texture")
                .on_hover_text("The texture the Block tool applies now.")
                .clicked()
            {
                use_now = Some(true);
            }
            ui.end_row();
            ui.label("Entity tool class");
            *dirty |= ui
                .add_sized(
                    [260.0, ui.spacing().interact_size.y],
                    egui::TextEdit::singleline(&mut editing.default_entity).hint_text(&defaults.default_entity),
                )
                .changed();
            if ui.button("Use the current class").clicked() {
                use_now = Some(false);
            }
            ui.end_row();
            ui.label("Recent textures listed");
            *dirty |= ui
                .add(egui::DragValue::new(&mut editing.recent_textures).range(1..=100))
                .changed();
            ui.end_row();
        });
    match use_now {
        Some(true) => editor.config.editing.default_material = editor.material.clone(),
        Some(false) => editor.config.editing.default_entity = editor.entity_class.clone(),
        None => return,
    }
    *dirty = true;
}

fn texture_browser_section(ui: &mut egui::Ui, editor: &mut Editor, dirty: &mut bool) {
    heading(
        ui,
        "Texture browser",
        "How the texture browser scrolls and loads its thumbnails.",
    );
    let view = &mut editor.config.view;
    let browser = &mut editor.config.texture_browser;
    ui.spacing_mut().slider_width = 260.0;
    egui::Grid::new("texture_browser_options")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Scrolling");
            let mut percent = (view.texture_scroll * 100.0).round();
            let slider = egui::Slider::new(&mut percent, 25.0..=200.0)
                .step_by(25.0)
                .suffix("% of a card per notch");
            if ui.add(slider).changed() {
                view.texture_scroll = percent / 100.0;
                *dirty = true;
            }
            ui.end_row();
            ui.label("Thumbnail size")
                .on_hover_text("Larger thumbnails look sharper on big cards but load slower and use more memory.");
            egui::ComboBox::from_id_salt("thumbnail_size")
                .selected_text(format!("{} px", browser.thumbnail_size))
                .show_ui(ui, |ui| {
                    for side in [128, 256, 512, 1024] {
                        *dirty |= ui
                            .selectable_value(&mut browser.thumbnail_size, side, format!("{side} px"))
                            .changed();
                    }
                });
            ui.end_row();
            ui.label("Loading threads");
            *dirty |= ui.add(egui::Slider::new(&mut browser.threads, 1..=16)).changed();
            ui.end_row();
            ui.label("Thumbnails kept in memory")
                .on_hover_text("Past this, thumbnails are dropped and load again when they scroll back into view.");
            *dirty |= ui
                .add(
                    egui::Slider::new(&mut browser.cache, 500..=20_000)
                        .logarithmic(true)
                        .step_by(100.0),
                )
                .changed();
            ui.end_row();
        });
    ui.add_space(8.0);
    if ui
        .button(format!(
            "{} Reset texture browser options",
            icon::ARROW_COUNTER_CLOCKWISE
        ))
        .clicked()
    {
        *browser = config::TextureBrowserOptions::default();
        view.texture_scroll = config::ViewOptions::default().texture_scroll;
        *dirty = true;
    }
}

fn interface_section(ui: &mut egui::Ui, editor: &mut Editor, dirty: &mut bool) {
    heading(ui, "Interface", "How Mallet's window looks and draws.");
    let interface = &mut editor.config.interface;
    ui.spacing_mut().slider_width = 260.0;
    egui::Grid::new("interface_options")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Interface size")
                .on_hover_text("Scales menus, panels and text. Applied once you let go.");
            let mut percent = (interface.scale * 100.0).round();
            let slider = egui::Slider::new(&mut percent, 50.0..=300.0).step_by(5.0).suffix("%");
            if ui.add(slider).changed() {
                interface.scale = percent / 100.0;
                *dirty = true;
            }
            ui.end_row();
            ui.label("Vertical sync");
            *dirty |= ui
                .checkbox(&mut interface.vsync, "Wait for the screen's refresh")
                .on_hover_text(
                    "Off draws as fast as the computer can, for the lowest input delay. Takes effect when Mallet \
                     next starts; MALLET_PRESENT overrides it.",
                )
                .changed();
            ui.end_row();
        });
}

/// A folder path that can be typed or browsed for; empty means `automatic`. Returns
/// whether it changed.
fn folder_field(ui: &mut egui::Ui, folder: &mut Option<PathBuf>, automatic: &str) -> bool {
    let mut text = folder
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let mut changed = false;
    ui.horizontal(|ui| {
        if ui
            .add_sized(
                [360.0, ui.spacing().interact_size.y],
                egui::TextEdit::singleline(&mut text).hint_text(automatic),
            )
            .changed()
        {
            *folder = (!text.trim().is_empty()).then(|| PathBuf::from(text.trim()));
            changed = true;
        }
        if ui.button(icon::FOLDER_OPEN).on_hover_text("Browse").clicked() {
            let start = folder.clone().unwrap_or_else(|| PathBuf::from(automatic));
            if let Some(picked) = rfd::FileDialog::new().set_directory(start).pick_folder() {
                *folder = Some(picked);
                changed = true;
            }
        }
    });
    changed
}

fn files_section(ui: &mut egui::Ui, editor: &mut Editor, dirty: &mut bool) {
    heading(
        ui,
        "Files",
        "Maps can be saved as VMF, which Hammer and the compilers read, or as .mallet, Mallet's own format: \
         the same map, about a tenth of the size and several times faster to open and save.",
    );
    ui.label("Save new maps as");
    ui.horizontal(|ui| {
        let files = &mut editor.config.files;
        *dirty |= ui
            .radio_value(&mut files.new_maps_as_mallet, true, ".mallet (Mallet Map)")
            .changed();
        *dirty |= ui
            .radio_value(&mut files.new_maps_as_mallet, false, ".vmf (Valve Map File)")
            .changed();
    });
    ui.label(
        RichText::new("Save As still offers both. Compiling a .mallet map writes a VMF copy for the compilers.")
            .color(MUTED),
    );
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("File > Recent Maps keeps");
        let files = &mut editor.config.files;
        let slider = egui::DragValue::new(&mut files.recent_maps)
            .range(1..=config::MAX_RECENT_MAPS)
            .suffix(" maps");
        if ui.add(slider).changed() {
            let keep = files.recent_map_count();
            editor.config.recent_maps.truncate(keep);
            *dirty = true;
        }
    });

    ui.add_space(14.0);
    ui.strong("Windows file types");
    ui.label(
        RichText::new(
            "For your Windows account only; nothing else changes. While Mallet runs, opening a map from \
             Explorer adds it as a tab instead of starting another Mallet.",
        )
        .color(MUTED),
    );
    ui.add_space(4.0);
    let types = shell::file_types();
    let mallet = file_type_row(
        ui,
        "Open .mallet maps in Mallet",
        "Double-clicking a .mallet file opens it here, and Explorer shows Mallet's icon for it.",
        &types.mallet,
    );
    let vmf = file_type_row(
        ui,
        "Mallet in \"Open with\" for VMF files",
        "Right-click a VMF and choose Open with > Mallet. Hammer stays the program VMF files open in.",
        &types.vmf_open_with,
    );
    let result = match (mallet, vmf) {
        (Some(true), _) => Some(shell::register_mallet()),
        (Some(false), _) => Some(shell::unregister_mallet()),
        (_, Some(true)) => Some(shell::add_vmf_open_with()),
        (_, Some(false)) => Some(shell::remove_vmf_open_with()),
        (None, None) => None,
    };
    match result {
        Some(Ok(())) => editor.message = "Updated the Windows file types".to_owned(),
        Some(Err(error)) => editor.message = format!("Could not update the Windows file types: {error}"),
        None => {}
    }
}

/// One file type: whether it is set up, and a button to set it up or undo it. Returns
/// `Some(true)` to set it up for this copy of Mallet, `Some(false)` to undo it.
fn file_type_row(ui: &mut egui::Ui, title: &str, note: &str, state: &shell::Registration) -> Option<bool> {
    let mut choice = None;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let (glyph, color, text) = match state {
            shell::Registration::ThisCopy => (icon::CHECK_CIRCLE, GOOD, "On".to_owned()),
            shell::Registration::OtherCopy(path) => {
                (icon::WARNING_CIRCLE, WARNING, format!("Set up for {}", path.display()))
            }
            shell::Registration::No => (icon::CIRCLE, MUTED, "Off".to_owned()),
        };
        ui.label(RichText::new(glyph).color(color));
        ui.strong(title);
        ui.label(RichText::new(text).color(color));
    });
    ui.label(RichText::new(note).color(MUTED));
    ui.horizontal(|ui| match state {
        shell::Registration::ThisCopy => {
            if ui.button("Turn off").clicked() {
                choice = Some(false);
            }
        }
        shell::Registration::OtherCopy(_) => {
            if ui
                .button("Use this copy of Mallet")
                .on_hover_text(
                    std::env::current_exe()
                        .map(|exe| exe.display().to_string())
                        .unwrap_or_default(),
                )
                .clicked()
            {
                choice = Some(true);
            }
            if ui.button("Turn off").clicked() {
                choice = Some(false);
            }
        }
        shell::Registration::No => {
            if ui.button("Turn on").clicked() {
                choice = Some(true);
            }
        }
    });
    choice
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

fn agents_section(ui: &mut egui::Ui, editor: &mut Editor, mcp_url: Option<&str>, dirty: &mut bool) {
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
    ui.add_space(8.0);
    let agents = &mut editor.config.agents;
    *dirty |= ui
        .checkbox(&mut agents.enabled, "Run the MCP server")
        .on_hover_text("Also lets a map opened from Explorer join this Mallet as a tab.")
        .changed();
    ui.add_enabled_ui(agents.enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label("Port");
            *dirty |= ui
                .add(egui::DragValue::new(&mut agents.port).range(1024..=65535))
                .changed();
            ui.label(RichText::new(format!("on 127.0.0.1, {} by default", crate::mcp::DEFAULT_PORT)).color(MUTED));
        });
    });
    ui.label(
        RichText::new(
            "Changes take effect when Mallet next starts. MALLET_MCP_PORT, when set, overrides the port, and 0 \
             turns the server off.",
        )
        .color(MUTED),
    );
    let url = mcp_url
        .map(str::to_owned)
        .unwrap_or_else(|| format!("http://127.0.0.1:{}/mcp", agents.port));
    let config = format!(r#"{{ "mcpServers": {{ "mallet": {{ "type": "http", "url": "{url}" }} }} }}"#);
    ui.add_space(6.0);
    ui.label("Client configuration (.mcp.json):");
    ui.label(RichText::new(&config).monospace().small());
    if ui.small_button(format!("{} Copy", icon::CLIPBOARD_TEXT)).clicked() {
        ui.ctx().copy_text(config);
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

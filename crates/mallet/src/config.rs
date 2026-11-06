//! Settings kept between runs, stored as JSON in `%APPDATA%\Mallet\config.json`, and the
//! programs Mallet runs to compile maps.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Game folders tried, in order, inside each Steam library when no game is configured.
const KNOWN_GAMES: [&str; 5] = [
    "GarrysMod/garrysmod",
    "Half-Life 2/hl2",
    "Source SDK Base 2013 Singleplayer/hl2",
    "Source SDK Base 2013 Multiplayer/hl2mp",
    "Counter-Strike Source/cstrike",
];
/// Where Steam is installed unless Mallet Options says otherwise.
pub const DEFAULT_STEAM_DIR: &str = "C:/Program Files (x86)/Steam";

/// Settings kept between runs. Missing fields take their defaults, so older files load.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The game folder holding `gameinfo.txt`, such as `.../GarrysMod/garrysmod`.
    pub game_dir: Option<PathBuf>,
    /// Steam's install folder, searched with its libraries for a game to use; `None` for
    /// the usual place.
    pub steam_dir: Option<PathBuf>,
    /// FGD files chosen instead of the game's default one; empty to use the default.
    pub fgds: Vec<PathBuf>,
    /// Games listed in Garry's Mod's `cfg/mount.cfg` that Mallet leaves unmounted.
    pub unmounted_games: Vec<String>,
    /// Maps opened or saved lately, the latest first.
    pub recent_maps: Vec<PathBuf>,
    /// Programs chosen for each compile tool; a missing one is found automatically.
    pub tools: ToolPaths,
    pub compile: CompileOptions,
    pub view: ViewOptions,
    pub colors: ColorOptions,
    pub editing: EditingOptions,
    pub files: FileOptions,
    pub texture_browser: TextureBrowserOptions,
    pub interface: InterfaceOptions,
    pub agents: AgentOptions,
}

/// How maps are saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FileOptions {
    /// Save As offers Mallet's own `.mallet` format first for a map that has never been
    /// saved; when off, it offers VMF first.
    pub new_maps_as_mallet: bool,
    /// How many maps File > Recent Maps keeps.
    pub recent_maps: usize,
}

impl Default for FileOptions {
    fn default() -> Self {
        Self {
            new_maps_as_mallet: true,
            recent_maps: 20,
        }
    }
}

impl FileOptions {
    /// The recent map count, kept between 1 and [`MAX_RECENT_MAPS`].
    pub fn recent_map_count(&self) -> usize {
        self.recent_maps.clamp(1, MAX_RECENT_MAPS)
    }
}

/// What new brushes, entities and lists start with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditingOptions {
    /// The material new brushes get when Mallet starts.
    pub default_material: String,
    /// The class the Entity tool creates when Mallet starts.
    pub default_entity: String,
    /// How many recently applied textures the texture tool lists.
    pub recent_textures: usize,
}

impl Default for EditingOptions {
    fn default() -> Self {
        Self {
            default_material: crate::editor::DEFAULT_MATERIAL.to_owned(),
            default_entity: crate::editor::ENTITY_CLASSES[0].to_owned(),
            recent_textures: 24,
        }
    }
}

impl EditingOptions {
    /// The default material, or Mallet's own when the setting is blank.
    pub fn material(&self) -> String {
        non_blank(&self.default_material, crate::editor::DEFAULT_MATERIAL)
    }

    /// The default entity class, or Mallet's own when the setting is blank.
    pub fn entity(&self) -> String {
        non_blank(&self.default_entity, crate::editor::ENTITY_CLASSES[0])
    }

    /// The recent texture count, kept between 1 and 100.
    pub fn recent_texture_count(&self) -> usize {
        self.recent_textures.clamp(1, 100)
    }
}

/// How the texture browser loads its thumbnails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextureBrowserOptions {
    /// Largest side of a thumbnail in pixels; larger looks sharper but loads slower.
    pub thumbnail_size: u32,
    /// Background threads loading thumbnails.
    pub threads: usize,
    /// Thumbnails kept in memory before they are dropped and reloaded as needed.
    pub cache: usize,
}

impl Default for TextureBrowserOptions {
    fn default() -> Self {
        Self {
            thumbnail_size: 256,
            threads: 4,
            cache: 2500,
        }
    }
}

impl TextureBrowserOptions {
    /// The thumbnail size, kept between 64 and 1024 pixels.
    pub fn thumbnail_side(&self) -> u32 {
        self.thumbnail_size.clamp(64, 1024)
    }

    /// The thread count, kept between 1 and 32.
    pub fn thread_count(&self) -> usize {
        self.threads.clamp(1, 32)
    }

    /// The cache size, kept between 200 and 50000 thumbnails.
    pub fn cache_size(&self) -> usize {
        self.cache.clamp(200, 50_000)
    }
}

/// How the window itself looks and draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InterfaceOptions {
    /// Size of menus, panels and text, where 1 is normal.
    pub scale: f32,
    /// Wait for the screen's refresh before showing each frame. Takes effect on restart;
    /// `MALLET_PRESENT` overrides it.
    pub vsync: bool,
}

impl Default for InterfaceOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            vsync: true,
        }
    }
}

impl InterfaceOptions {
    /// The interface scale, kept between 0.5 and 3.
    pub fn zoom(&self) -> f32 {
        if self.scale.is_finite() {
            self.scale.clamp(0.5, 3.0)
        } else {
            1.0
        }
    }
}

/// The MCP server AI agents connect to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentOptions {
    /// Run the server. Takes effect on restart.
    pub enabled: bool,
    /// The port it listens on, on 127.0.0.1. Takes effect on restart.
    pub port: u16,
}

impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            port: crate::mcp::DEFAULT_PORT,
        }
    }
}

/// A colour of the views, as red, green and blue from 0 to 255.
pub type Rgb = [u8; 3];

/// The colours the viewports are drawn with, Hammer's by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorOptions {
    /// Behind everything in every view.
    pub background: Rgb,
    /// Ordinary grid lines in the 2D views.
    pub grid: Rgb,
    /// Grid lines at every highlight spacing.
    pub grid_highlight: Rgb,
    /// Grid lines at every major spacing.
    pub grid_major: Rgb,
    /// The lines through the world origin.
    pub axis: Rgb,
    /// Selected brushes and entities.
    pub selection: Rgb,
    /// The Block tool's box, the entity marker and the selection marquee.
    pub tool: Rgb,
    /// The box and handles around the selection.
    pub handles: Rgb,
    /// The outline of faces the texture tool has selected.
    pub selected_faces: Rgb,
}

impl Default for ColorOptions {
    fn default() -> Self {
        Self {
            background: [0, 0, 0],
            grid: [38, 38, 38],
            grid_highlight: [65, 65, 65],
            grid_major: [100, 46, 0],
            axis: [0, 100, 100],
            selection: [255, 0, 0],
            tool: [255, 255, 255],
            handles: [255, 255, 255],
            selected_faces: [255, 210, 60],
        }
    }
}

/// `value` trimmed, or `fallback` when it is blank.
fn non_blank(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() { fallback } else { value }.to_owned()
}

/// Programs chosen in Mallet Options. `None` means "find it automatically".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPaths {
    pub vbsp: Option<PathBuf>,
    pub vvis: Option<PathBuf>,
    pub vrad: Option<PathBuf>,
    pub vbspinfo: Option<PathBuf>,
    pub bspzip: Option<PathBuf>,
    pub sourcecubemap: Option<PathBuf>,
    pub game: Option<PathBuf>,
}

impl ToolPaths {
    pub fn get(&self, tool: ToolKind) -> Option<&PathBuf> {
        match tool {
            ToolKind::Vbsp => self.vbsp.as_ref(),
            ToolKind::Vvis => self.vvis.as_ref(),
            ToolKind::Vrad => self.vrad.as_ref(),
            ToolKind::Vbspinfo => self.vbspinfo.as_ref(),
            ToolKind::Bspzip => self.bspzip.as_ref(),
            ToolKind::SourceCubemap => self.sourcecubemap.as_ref(),
            ToolKind::Game => self.game.as_ref(),
        }
    }

    pub fn set(&mut self, tool: ToolKind, path: Option<PathBuf>) {
        let slot = match tool {
            ToolKind::Vbsp => &mut self.vbsp,
            ToolKind::Vvis => &mut self.vvis,
            ToolKind::Vrad => &mut self.vrad,
            ToolKind::Vbspinfo => &mut self.vbspinfo,
            ToolKind::Bspzip => &mut self.bspzip,
            ToolKind::SourceCubemap => &mut self.sourcecubemap,
            ToolKind::Game => &mut self.game,
        };
        *slot = path;
    }
}

/// How compiles behave.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompileOptions {
    /// Save the map before compiling it, so the compile sees the latest changes.
    pub save_before: bool,
    /// Run the compilers below normal priority so the editor stays responsive.
    pub low_priority: bool,
    /// Flash the taskbar button when a compile ends.
    pub notify: bool,
    /// Show the leak line in the views when VBSP finds a leak.
    pub load_pointfile: bool,
    /// Keep each compile's log in `%APPDATA%\Mallet\logs`.
    pub keep_logs: bool,
    /// The folder beside a `.mallet` map that its VMF copy, BSP and logs are compiled in.
    pub compile_folder: String,
    /// Lines of compiler output the compile window keeps; older ones are dropped.
    pub output_lines: usize,
    /// The preset the compile window starts with and Shift+F9 runs.
    pub last_preset: String,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            save_before: true,
            low_priority: true,
            notify: true,
            load_pointfile: true,
            keep_logs: true,
            compile_folder: crate::compile::COMPILE_FOLDER.to_owned(),
            output_lines: 50_000,
            last_preset: "Normal".to_owned(),
        }
    }
}

impl CompileOptions {
    /// The compile folder's name, or Mallet's own when the setting is blank or names
    /// anything but a plain folder beside the map.
    pub fn folder_name(&self) -> String {
        let name = self.compile_folder.trim();
        let plain = !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != "..";
        if plain { name } else { crate::compile::COMPILE_FOLDER }.to_owned()
    }

    /// The output line limit, kept between 1000 and 1000000.
    pub fn line_limit(&self) -> usize {
        self.output_lines.clamp(1000, 1_000_000)
    }
}

/// How the viewports look and move.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewOptions {
    pub net_graph: bool,
    /// How far the 3D view draws, in units.
    pub draw_distance: f64,
    /// Horizontal field of view of the 3D view, in degrees.
    pub fov: f64,
    /// 3D camera speed in units per second.
    pub camera_speed: f64,
    /// Degrees turned per point of mouse movement in mouselook.
    pub look_sensitivity: f64,
    /// Grid size of new maps.
    pub default_grid: f64,
    /// How far one mouse wheel notch scrolls the texture browser, as a share of a card's
    /// height, from 0.25 to 2.
    pub texture_scroll: f64,
    /// Degrees per second the arrow keys turn the 3D camera.
    pub turn_speed: f64,
    /// How quickly the 3D camera reaches its speed and stops, per second; higher is
    /// snappier.
    pub camera_acceleration: f64,
    /// Units one mouse wheel notch moves the 3D camera.
    pub wheel_step: f64,
    /// Frames of mouse movement mouselook averages over; 1 turns smoothing off.
    pub look_smoothing: usize,
    /// How much one mouse wheel notch zooms a 2D view.
    pub zoom_step: f64,
    /// Grid lines closer than this many pixels are thinned out.
    pub grid_min_pixels: f64,
    /// Units between highlighted grid lines; 0 for none.
    pub grid_highlight: f64,
    /// Units between major grid lines; 0 for none.
    pub grid_major: f64,
    /// How close, in pixels, a click must be to a brush's outline to select it.
    pub pick_tolerance: f64,
}

impl Default for ViewOptions {
    fn default() -> Self {
        Self {
            net_graph: true,
            draw_distance: crate::view3d::DEFAULT_DRAW_DISTANCE,
            fov: crate::view3d::DEFAULT_FOV,
            camera_speed: crate::view3d::DEFAULT_MOVE_SPEED,
            look_sensitivity: crate::view3d::DEFAULT_LOOK_SENSITIVITY,
            default_grid: 64.0,
            texture_scroll: 1.0,
            turn_speed: crate::view3d::DEFAULT_TURN_SPEED,
            camera_acceleration: crate::view3d::DEFAULT_ACCELERATION,
            wheel_step: crate::view3d::DEFAULT_WHEEL_STEP,
            look_smoothing: crate::view3d::DEFAULT_LOOK_FRAMES,
            zoom_step: crate::view2d::DEFAULT_ZOOM_STEP,
            grid_min_pixels: 4.0,
            grid_highlight: 64.0,
            grid_major: 1024.0,
            pick_tolerance: 4.0,
        }
    }
}

impl Config {
    /// Loads the settings, or defaults when there are none yet or they cannot be read.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        if path.is_file() || profile().is_none() {
            return Self::load_from(&path);
        }
        // A new profile starts from the main settings, without their map history.
        let main = std::env::var_os("APPDATA").map(|appdata| Path::new(&appdata).join("Mallet").join("config.json"));
        let mut config = main.map_or_else(Self::default, |main| Self::load_from(&main));
        config.recent_maps.clear();
        config
    }

    /// Puts a map at the top of the recent list, keeping it short and free of repeats.
    pub fn remember_map(&mut self, path: &Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
        let key = |path: &Path| path.to_string_lossy().replace('/', "\\").to_lowercase();
        let wanted = key(&path);
        self.recent_maps.retain(|recent| key(recent) != wanted);
        self.recent_maps.insert(0, path);
        self.recent_maps.truncate(self.files.recent_map_count());
    }

    /// Loads settings from a file, or defaults when it is missing or unreadable.
    pub fn load_from(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the settings, creating the folder if needed.
    pub fn save(&self) -> Result<(), String> {
        let path = config_path().ok_or("APPDATA is not set, so settings cannot be saved")?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        fs::write(path, text).map_err(|error| error.to_string())
    }
}

/// The most maps File > Recent Maps can be set to keep.
pub const MAX_RECENT_MAPS: usize = 50;

/// The `MALLET_PROFILE` in use, if any: a separate set of settings and layouts, as test
/// copies use.
pub fn profile() -> Option<String> {
    std::env::var("MALLET_PROFILE")
        .ok()
        .map(|profile| profile.trim().to_owned())
        .filter(|profile| !profile.is_empty())
}

/// The name of Mallet's folder in `%APPDATA%`: `Mallet`, or `Mallet-<profile>`.
pub fn app_folder() -> String {
    match profile() {
        Some(profile) => format!("Mallet-{profile}"),
        None => "Mallet".to_owned(),
    }
}

/// Mallet's folder in `%APPDATA%`, holding settings, presets and compile logs.
pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| Path::new(&appdata).join(app_folder()))
}

fn config_path() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("config.json"))
}

/// A program Mallet runs while compiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolKind {
    Vbsp,
    Vvis,
    Vrad,
    Vbspinfo,
    Bspzip,
    SourceCubemap,
    Game,
}

impl ToolKind {
    pub const ALL: [ToolKind; 7] = [
        ToolKind::Vbsp,
        ToolKind::Vvis,
        ToolKind::Vrad,
        ToolKind::Vbspinfo,
        ToolKind::Bspzip,
        ToolKind::SourceCubemap,
        ToolKind::Game,
    ];

    /// The name shown in Mallet Options.
    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Vbsp => "VBSP",
            ToolKind::Vvis => "VVIS",
            ToolKind::Vrad => "VRAD",
            ToolKind::Vbspinfo => "VBSPInfo",
            ToolKind::Bspzip => "BSPZip",
            ToolKind::SourceCubemap => "SourceCubemap",
            ToolKind::Game => "Game",
        }
    }

    /// The name used in settings and MCP tools.
    pub fn key(self) -> &'static str {
        match self {
            ToolKind::Vbsp => "vbsp",
            ToolKind::Vvis => "vvis",
            ToolKind::Vrad => "vrad",
            ToolKind::Vbspinfo => "vbspinfo",
            ToolKind::Bspzip => "bspzip",
            ToolKind::SourceCubemap => "sourcecubemap",
            ToolKind::Game => "game",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ToolKind::Vbsp => "Turns the VMF into a BSP: brushes, entities and props.",
            ToolKind::Vvis => "Works out what can be seen from where, so the engine draws less.",
            ToolKind::Vrad => "Bakes the lighting.",
            ToolKind::Vbspinfo => "Prints what a compiled map contains and how close it is to the limits.",
            ToolKind::Bspzip => "Packs files into a BSP.",
            ToolKind::SourceCubemap => "Builds cubemaps on the graphics card without starting the game.",
            ToolKind::Game => "The game to test the map in.",
        }
    }

    /// Every existing program that could be this tool, best first: the game's 64-bit
    /// tools, its 32-bit ones and the Hammer++ builds, or for SourceCubemap and the game,
    /// the usual install places.
    pub fn candidates(self, game_dir: Option<&Path>) -> Vec<PathBuf> {
        let root = game_dir.and_then(Path::parent);
        let mut found: Vec<PathBuf> = Vec::new();
        let mut add = |path: PathBuf| {
            // Rebuilding from components gives one kind of slash on Windows.
            let path: PathBuf = path.components().collect();
            if path.is_file() && !found.contains(&path) {
                found.push(path);
            }
        };
        match self {
            ToolKind::SourceCubemap => {
                for folder in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
                    add(folder.join("sourcecubemap.exe"));
                }
                if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                    add(Path::new(&local).join("Programs/SourceCubemap/sourcecubemap.exe"));
                }
                for program_files in ["ProgramFiles", "ProgramFiles(x86)"] {
                    if let Some(folder) = std::env::var_os(program_files) {
                        add(Path::new(&folder).join("SourceCubemap/sourcecubemap.exe"));
                    }
                }
                if let Some(home) = std::env::var_os("USERPROFILE") {
                    add(Path::new(&home).join("SourceCubemap/sourcecubemap.exe"));
                }
                for drive in ["C:", "D:", "E:"] {
                    add(PathBuf::from(format!(
                        "{drive}/Github/SourceCubemap/build/release/sourcecubemap.exe"
                    )));
                    add(PathBuf::from(format!("{drive}/SourceCubemap/sourcecubemap.exe")));
                }
            }
            ToolKind::Game => {
                let Some(root) = root else { return found };
                for name in ["gmod.exe", "hl2.exe", "hl2_win64.exe", "bin/win64/gmod.exe", "csgo.exe"] {
                    add(root.join(name));
                }
                if let Some(game) = game_dir.and_then(Path::file_name) {
                    add(root.join(format!("{}.exe", game.to_string_lossy())));
                }
            }
            _ => {
                let Some(root) = root else { return found };
                let name = self.key();
                for folder in ["bin/win64", "bin"] {
                    add(root.join(folder).join(format!("{name}.exe")));
                }
                for folder in ["bin/win64", "bin"] {
                    add(root.join(folder).join(format!("{name}plusplus.exe")));
                }
            }
        }
        found
    }
}

/// Looks for an installed Source game: the first known game folder found in the Steam
/// libraries listed by the Steam install at `steam_dir`, or in the usual place, in the
/// order Steam lists them.
pub fn detect_game_dir(steam_dir: Option<&Path>) -> Option<PathBuf> {
    let steam = steam_dir.map_or_else(|| PathBuf::from(DEFAULT_STEAM_DIR), Path::to_path_buf);
    let mut libraries = vec![steam.clone()];
    let listing = fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")).unwrap_or_default();
    libraries.extend(
        listing
            .lines()
            .filter(|line| line.trim_start().starts_with("\"path\""))
            .filter_map(|line| line.split('"').nth(3))
            .map(|path| PathBuf::from(path.replace("\\\\", "\\"))),
    );
    libraries.dedup();
    libraries.iter().find_map(|library| {
        KNOWN_GAMES
            .iter()
            .map(|game| library.join("steamapps").join("common").join(game))
            .find(|dir| dir.join("gameinfo.txt").is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_files_load_with_defaults_and_round_trip() {
        let folder = std::env::temp_dir().join(format!("mallet-config-{}", std::process::id()));
        let path = folder.join("config.json");
        fs::create_dir_all(&folder).unwrap();
        fs::write(&path, r#"{ "game_dir": "C:/Games/hl2", "fgds": ["a.fgd"] }"#).unwrap();
        let mut config = Config::load_from(&path);
        assert_eq!(config.game_dir, Some(PathBuf::from("C:/Games/hl2")));
        assert_eq!(config.fgds, [PathBuf::from("a.fgd")]);
        assert!(config.compile.save_before);
        assert_eq!(config.tools.get(ToolKind::Vrad), None);

        config
            .tools
            .set(ToolKind::SourceCubemap, Some(PathBuf::from("D:/scm.exe")));
        for map in ["C:/maps/a.vmf", "C:/maps/b.vmf", "C:\\MAPS\\A.vmf"] {
            config.remember_map(Path::new(map));
        }
        assert_eq!(
            config.recent_maps.len(),
            2,
            "the same map in another case or slash is one map"
        );
        assert!(config.recent_maps[0]
            .to_string_lossy()
            .to_lowercase()
            .ends_with("a.vmf"));
        config.view.camera_speed = 500.0;
        config.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path), config);
        fs::remove_dir_all(folder).unwrap();
    }
}

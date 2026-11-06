//! Settings kept between runs, stored as JSON in `%APPDATA%\Mallet\config.json`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Game folders tried, in order, inside each Steam library when no game is configured.
const KNOWN_GAMES: [&str; 5] = [
    "GarrysMod/garrysmod",
    "Half-Life 2/hl2",
    "Source SDK Base 2013 Singleplayer/hl2",
    "Source SDK Base 2013 Multiplayer/hl2mp",
    "Counter-Strike Source/cstrike",
];
const DEFAULT_STEAM_DIR: &str = "C:/Program Files (x86)/Steam";

/// Settings kept between runs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Config {
    /// The game folder holding `gameinfo.txt`, such as `.../GarrysMod/garrysmod`.
    pub game_dir: Option<PathBuf>,
    /// FGD files chosen instead of the game's default one; empty to use the default.
    pub fgds: Vec<PathBuf>,
}

impl Config {
    /// Loads the settings, or defaults when there are none yet or they cannot be read.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Self {
            game_dir: value.get("game_dir").and_then(Value::as_str).map(PathBuf::from),
            fgds: value
                .get("fgds")
                .and_then(Value::as_array)
                .map(|paths| paths.iter().filter_map(Value::as_str).map(PathBuf::from).collect())
                .unwrap_or_default(),
        }
    }

    /// Writes the settings, creating the folder if needed.
    pub fn save(&self) -> Result<(), String> {
        let path = config_path().ok_or("APPDATA is not set, so settings cannot be saved")?;
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        }
        let value = json!({
            "game_dir": self.game_dir.as_ref().map(|dir| dir.display().to_string()),
            "fgds": self.fgds.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        });
        let text = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
        fs::write(path, text).map_err(|error| error.to_string())
    }
}

fn config_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| Path::new(&appdata).join("Mallet").join("config.json"))
}

/// Looks for an installed Source game: the first known game folder found in the Steam
/// libraries listed by Steam, in the order Steam lists them.
pub fn detect_game_dir() -> Option<PathBuf> {
    let steam = PathBuf::from(DEFAULT_STEAM_DIR);
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

//! A game's virtual file system, built from the search paths in its `gameinfo.txt`.

use std::fs;
use std::path::{Path, PathBuf};

use mallet_kv::{Block, Item};

use crate::vpk::{normalize, VpkArchive};
use crate::ContentError;

/// One mounted location.
#[derive(Debug)]
enum Mount {
    Folder(PathBuf),
    Vpk(VpkArchive),
}

/// Game files from every search path, looked up in priority order.
#[derive(Debug)]
pub struct GameFs {
    game_dir: PathBuf,
    mounts: Vec<Mount>,
}

impl GameFs {
    /// Mounts the `game` search paths of the `gameinfo.txt` in `game_dir`, such as
    /// `.../GarrysMod/garrysmod` or `.../Half-Life 2/hl2`.
    ///
    /// Search paths that do not exist are skipped, as the engine does. Paths ending in
    /// `.vpk` mount the matching `_dir.vpk`; paths ending in `/*` mount every subfolder.
    pub fn from_gameinfo(game_dir: &Path) -> Result<Self, ContentError> {
        let gameinfo_path = game_dir.join("gameinfo.txt");
        if !gameinfo_path.is_file() {
            return Err(ContentError::NoGameInfo(game_dir.to_owned()));
        }
        let text = fs::read(&gameinfo_path).map_err(|error| ContentError::io(&gameinfo_path, error))?;
        let text = String::from_utf8_lossy(&text);
        let items = mallet_kv::parse(&text).map_err(|error| ContentError::Syntax {
            path: gameinfo_path.clone(),
            error,
        })?;
        let base_dir = game_dir.parent().unwrap_or(game_dir).to_owned();
        let mut filesystem = Self {
            game_dir: game_dir.to_owned(),
            mounts: Vec::new(),
        };
        for value in search_paths(&items) {
            filesystem.mount_search_path(&value, game_dir, &base_dir);
        }
        Ok(filesystem)
    }

    /// The folder holding `gameinfo.txt`.
    pub fn game_dir(&self) -> &Path {
        &self.game_dir
    }

    /// Short descriptions of every mount, in priority order.
    pub fn mounts(&self) -> Vec<String> {
        self.mounts
            .iter()
            .map(|mount| match mount {
                Mount::Folder(path) => format!("folder {}", path.display()),
                Mount::Vpk(archive) => format!("vpk ({} files)", archive.len()),
            })
            .collect()
    }

    /// Reads a game file such as `materials/tools/toolsnodraw.vmt` from the first mount
    /// that has it. Paths ignore case and may use either slash.
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let path = normalize(path);
        self.mounts.iter().find_map(|mount| match mount {
            Mount::Folder(folder) => fs::read(folder.join(&path)).ok(),
            Mount::Vpk(archive) => archive.read(&path).ok().flatten(),
        })
    }

    fn mount_search_path(&mut self, value: &str, game_dir: &Path, base_dir: &Path) {
        let expanded = value
            .replace("|gameinfo_path|", &format!("{}/", game_dir.display()))
            .replace("|all_source_engine_paths|", &format!("{}/", base_dir.display()));
        let path = Path::new(&expanded);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            base_dir.join(path)
        };
        let text = path.to_string_lossy().replace('\\', "/");

        if let Some(parent) = text.strip_suffix("/*") {
            let Ok(children) = fs::read_dir(parent) else { return };
            let mut folders: Vec<PathBuf> = children
                .filter_map(Result::ok)
                .map(|child| child.path())
                .filter(|child| child.is_dir())
                .collect();
            folders.sort();
            for folder in folders {
                self.add(Mount::Folder(folder));
            }
        } else if let Some(stem) = text.strip_suffix(".vpk") {
            let dir_file = PathBuf::from(format!("{stem}_dir.vpk"));
            if let Ok(archive) = VpkArchive::open(&dir_file) {
                self.add(Mount::Vpk(archive));
            }
        } else if path.is_dir() {
            self.add(Mount::Folder(path.components().collect()));
        }
    }

    fn add(&mut self, mount: Mount) {
        if let Mount::Folder(new) = &mount {
            let duplicate = self
                .mounts
                .iter()
                .any(|existing| matches!(existing, Mount::Folder(folder) if same_folder(folder, new)));
            if duplicate {
                return;
            }
        }
        self.mounts.push(mount);
    }
}

fn same_folder(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The values of `game` search paths (`game`, `game+mod` and so on), in file order.
fn search_paths(items: &[Item]) -> Vec<String> {
    let Some(gameinfo) = items.iter().find_map(|item| match item {
        Item::Block(block) if block.name.eq_ignore_ascii_case("GameInfo") => Some(block),
        _ => None,
    }) else {
        return Vec::new();
    };
    let search_paths: Option<&Block> = gameinfo
        .blocks_named("FileSystem")
        .next()
        .and_then(|filesystem| filesystem.blocks_named("SearchPaths").next());
    search_paths
        .into_iter()
        .flat_map(Block::pairs)
        .filter(|pair| pair.key.split('+').any(|kind| kind.eq_ignore_ascii_case("game")))
        .map(|pair| pair.value.clone())
        .collect()
}

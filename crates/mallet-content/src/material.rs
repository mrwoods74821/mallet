//! Resolves material names to the texture Hammer shows for them.

use std::collections::HashMap;
use std::sync::Arc;

use mallet_kv::{Block, Item};

use crate::vpk::normalize;
use crate::vtf::{self, Image};
use crate::GameFs;

/// Largest side, in pixels, of the mip level decoded for viewports.
const MAX_TEXTURE_SIDE: u32 = 512;
/// How many `patch` materials may include each other before giving up.
const MAX_PATCH_DEPTH: usize = 8;

/// A texture ready for display. Shared, because many faces use the same one.
pub type Texture = Arc<Image>;

/// Loads and caches material textures from a game's files.
#[derive(Debug)]
pub struct Materials {
    filesystem: GameFs,
    cache: HashMap<String, Option<Texture>>,
}

impl Materials {
    pub fn new(filesystem: GameFs) -> Self {
        Self {
            filesystem,
            cache: HashMap::new(),
        }
    }

    pub fn filesystem(&self) -> &GameFs {
        &self.filesystem
    }

    /// The texture Hammer shows for a material such as `TOOLS/TOOLSNODRAW`, or `None`
    /// when the material or its texture cannot be found or decoded. Results are cached.
    pub fn texture(&mut self, material: &str) -> Option<Texture> {
        let key = material_key(material);
        if let Some(cached) = self.cache.get(&key) {
            return cached.clone();
        }
        let texture = self.load(&key);
        self.cache.insert(key, texture.clone());
        texture
    }

    /// Loads several materials' textures at once, spread over every CPU core, and returns
    /// them in the same order. Results are cached like [`Materials::texture`].
    pub fn textures(&mut self, materials: &[&str]) -> Vec<Option<Texture>> {
        let keys: Vec<String> = materials.iter().map(|material| material_key(material)).collect();
        let mut missing: Vec<&String> = keys.iter().filter(|key| !self.cache.contains_key(*key)).collect();
        missing.sort();
        missing.dedup();
        let loaded = parallel_map(&missing, |key| self.load(key));
        for (key, texture) in missing.into_iter().zip(loaded) {
            self.cache.insert(key.clone(), texture);
        }
        keys.iter().map(|key| self.cache.get(key).cloned().flatten()).collect()
    }

    fn load(&self, key: &str) -> Option<Texture> {
        let texture_name = self.texture_name(key, 0)?;
        let data = self.filesystem.read(&format!("materials/{texture_name}.vtf"))?;
        vtf::decode(&data, MAX_TEXTURE_SIDE).ok().map(Arc::new)
    }

    /// Finds the texture a material shows in the editor: `%tooltexture` first, as Hammer
    /// does for tool materials, then `$basetexture`. Follows `patch` materials to the
    /// material they include, applying their `insert` and `replace` overrides.
    fn texture_name(&self, key: &str, depth: usize) -> Option<String> {
        let data = self.filesystem.read(&format!("materials/{key}.vmt"))?;
        let text = String::from_utf8_lossy(&data);
        let items = mallet_kv::parse(&text).ok()?;
        let root = items.iter().find_map(|item| match item {
            Item::Block(block) => Some(block),
            Item::Pair(_) => None,
        })?;
        if root.name.eq_ignore_ascii_case("patch") {
            let overrides = ["replace", "insert"]
                .iter()
                .filter_map(|name| root.blocks_named(name).next())
                .find_map(editor_texture);
            if overrides.is_some() {
                return overrides;
            }
            if depth >= MAX_PATCH_DEPTH {
                return None;
            }
            return self.texture_name(&material_key(root.get("include")?), depth + 1);
        }
        editor_texture(root)
    }
}

/// Applies `work` to every item using one thread per CPU core, keeping the input order.
pub fn parallel_map<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(|| part.iter().map(&work).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().expect("texture worker panicked"))
            .collect()
    })
}

fn editor_texture(block: &Block) -> Option<String> {
    block
        .get("%tooltexture")
        .or_else(|| block.get("$basetexture"))
        .map(|name| normalize(name).trim_end_matches(".vtf").to_owned())
}

/// The cache key for a material: lower case, forward slashes, without the `materials/`
/// prefix or `.vmt` extension.
fn material_key(material: &str) -> String {
    let key = normalize(material);
    let key = key.strip_prefix("materials/").unwrap_or(&key);
    key.strip_suffix(".vmt").unwrap_or(key).to_owned()
}

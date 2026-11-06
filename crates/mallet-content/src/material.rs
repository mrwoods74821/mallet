//! Resolves material names to the texture Hammer shows for them and how it uses alpha.

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
/// Alpha below which `$alphatest` materials are cut out when they set no reference.
const DEFAULT_ALPHA_TEST_REFERENCE: f32 = 0.5;

/// A texture ready for display. Shared, because many faces use the same one.
pub type Texture = Arc<Image>;

/// How a material's texture alpha is used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaMode {
    /// Alpha is ignored.
    Opaque,
    /// Pixels with alpha below the reference are cut out (`$alphatest`).
    Test(f32),
    /// Blended over what is behind, with the texture's alpha times this opacity
    /// (`$translucent`, or `$alpha` below 1).
    Blend(f32),
}

/// What the editor needs to draw a material.
#[derive(Debug, Clone)]
pub struct Material {
    /// The texture, or `None` when the material or its texture cannot be found or decoded.
    pub texture: Option<Texture>,
    pub alpha: AlphaMode,
}

impl Material {
    fn missing() -> Self {
        Self {
            texture: None,
            alpha: AlphaMode::Opaque,
        }
    }
}

/// Loads and caches materials from a game's files.
#[derive(Debug)]
pub struct Materials {
    loader: MaterialLoader,
    cache: HashMap<String, Material>,
}

/// Loads materials without caching. Cheap to clone and safe to send to worker threads,
/// so textures can stream in the background.
#[derive(Debug, Clone)]
pub struct MaterialLoader {
    filesystem: Arc<GameFs>,
}

/// Material keys that matter to the editor, collected from a VMT and the materials it
/// patches. `None` means the key was not set.
#[derive(Debug, Default, Clone)]
struct Properties {
    texture: Option<String>,
    translucent: Option<bool>,
    alpha_test: Option<bool>,
    alpha_test_reference: Option<f32>,
    alpha: Option<f32>,
}

impl Properties {
    fn read(block: &Block) -> Self {
        let number = |key: &str| block.get(key).and_then(|value| value.trim().parse::<f32>().ok());
        let flag = |key: &str| number(key).map(|value| value != 0.0);
        Self {
            texture: block
                .get("%tooltexture")
                .or_else(|| block.get("$basetexture"))
                .map(|name| normalize(name).trim_end_matches(".vtf").to_owned()),
            translucent: flag("$translucent"),
            alpha_test: flag("$alphatest"),
            alpha_test_reference: number("$alphatestreference"),
            alpha: number("$alpha"),
        }
    }

    /// Takes every key `other` sets, keeping the rest.
    fn overridden_by(self, other: Self) -> Self {
        Self {
            texture: other.texture.or(self.texture),
            translucent: other.translucent.or(self.translucent),
            alpha_test: other.alpha_test.or(self.alpha_test),
            alpha_test_reference: other.alpha_test_reference.or(self.alpha_test_reference),
            alpha: other.alpha.or(self.alpha),
        }
    }

    fn alpha_mode(&self) -> AlphaMode {
        let opacity = self.alpha.unwrap_or(1.0).clamp(0.0, 1.0);
        if self.translucent == Some(true) || opacity < 1.0 {
            AlphaMode::Blend(opacity)
        } else if self.alpha_test == Some(true) {
            AlphaMode::Test(self.alpha_test_reference.unwrap_or(DEFAULT_ALPHA_TEST_REFERENCE))
        } else {
            AlphaMode::Opaque
        }
    }
}

impl Materials {
    pub fn new(filesystem: GameFs) -> Self {
        Self {
            loader: MaterialLoader {
                filesystem: Arc::new(filesystem),
            },
            cache: HashMap::new(),
        }
    }

    pub fn filesystem(&self) -> &GameFs {
        &self.loader.filesystem
    }

    /// A loader for worker threads, sharing this game's mounted files.
    pub fn loader(&self) -> MaterialLoader {
        self.loader.clone()
    }

    /// The texture Hammer shows for a material such as `TOOLS/TOOLSNODRAW`, or `None`
    /// when the material or its texture cannot be found or decoded. Results are cached.
    pub fn texture(&mut self, material: &str) -> Option<Texture> {
        self.materials(&[material]).pop().and_then(|material| material.texture)
    }

    /// Loads several materials at once, spread over every CPU core, and returns them in
    /// the same order. Results are cached.
    pub fn materials(&mut self, materials: &[&str]) -> Vec<Material> {
        let keys: Vec<String> = materials.iter().map(|material| material_key(material)).collect();
        let mut missing: Vec<&String> = keys.iter().filter(|key| !self.cache.contains_key(*key)).collect();
        missing.sort();
        missing.dedup();
        let loaded = parallel_map(&missing, |key| self.loader.load(key));
        for (key, material) in missing.into_iter().zip(loaded) {
            self.cache.insert(key.clone(), material);
        }
        keys.iter()
            .map(|key| self.cache.get(key).cloned().unwrap_or_else(Material::missing))
            .collect()
    }
}

impl MaterialLoader {
    /// Loads a studio model such as `models/props_c17/oildrum001.mdl`.
    pub fn model(&self, path: &str) -> Result<crate::model::Model, String> {
        crate::model::load_model(&self.filesystem, path)
    }

    /// Loads a material such as `TOOLS/TOOLSNODRAW`: its texture and alpha mode.
    pub fn load(&self, material: &str) -> Material {
        let key = material_key(material);
        let Some(properties) = self.properties(&key, 0) else {
            return Material::missing();
        };
        let texture = properties
            .texture
            .as_ref()
            .and_then(|name| self.filesystem.read(&format!("materials/{name}.vtf")))
            .and_then(|data| vtf::decode(&data, MAX_TEXTURE_SIDE).ok())
            .map(Arc::new);
        Material {
            texture,
            alpha: properties.alpha_mode(),
        }
    }

    /// Reads a material's properties. The texture is `%tooltexture` if set, as Hammer
    /// shows for tool materials, otherwise `$basetexture`. A `patch` material takes the
    /// properties of the material it includes and applies its `insert` and `replace`
    /// blocks on top.
    fn properties(&self, key: &str, depth: usize) -> Option<Properties> {
        let data = self.filesystem.read(&format!("materials/{key}.vmt"))?;
        let text = String::from_utf8_lossy(&data);
        let items = mallet_kv::parse(&text).ok()?;
        let root = items.iter().find_map(|item| match item {
            Item::Block(block) => Some(block),
            Item::Pair(_) => None,
        })?;
        if !root.name.eq_ignore_ascii_case("patch") {
            return Some(Properties::read(root));
        }
        let base = match root.get("include") {
            Some(include) if depth < MAX_PATCH_DEPTH => {
                self.properties(&material_key(include), depth + 1).unwrap_or_default()
            }
            _ => Properties::default(),
        };
        Some(
            ["insert", "replace"]
                .iter()
                .filter_map(|name| root.blocks_named(name).next())
                .map(Properties::read)
                .fold(base, Properties::overridden_by),
        )
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

/// The cache key for a material: lower case, forward slashes, without the `materials/`
/// prefix or `.vmt` extension.
fn material_key(material: &str) -> String {
    let key = normalize(material);
    let key = key.strip_prefix("materials/").unwrap_or(&key);
    key.strip_suffix(".vmt").unwrap_or(key).to_owned()
}

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

/// What a material's faces are to the compilers, from its `%compile` keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Surface {
    /// An ordinary surface.
    #[default]
    Normal,
    /// Sky (`%compilesky` or `%compile2Dsky`): the skybox shows through it and the sun
    /// shines in through it.
    Sky,
    /// Never drawn but solid, so it still blocks light: `%compilenodraw` and
    /// `%compileblocklight`.
    Nodraw,
    /// A tool face the compilers remove, invisible and letting light through: clip, hint,
    /// skip, trigger, origin, areaportal, occluder, ladder and the like.
    Tool,
}

/// `%compile` keys of tool materials whose faces the compilers remove.
const TOOL_KEYS: [&str; 14] = [
    "%compileclip",
    "%compilehint",
    "%compileskip",
    "%compiletrigger",
    "%compileorigin",
    "%compileareaportal",
    "%compileoccluder",
    "%compilenpcclip",
    "%playerclip",
    "%compileplayerclip",
    "%compileladder",
    "%compileinvisible",
    "%compilefog",
    "%compileblocklos",
];

/// What the editor needs to draw a material.
#[derive(Debug, Clone)]
pub struct Material {
    /// The texture, or `None` when the material or its texture cannot be found or decoded.
    pub texture: Option<Texture>,
    pub alpha: AlphaMode,
    pub surface: Surface,
    /// For `$selfillum` materials, the glow's tint (`$selfillumtint`); the texture's
    /// alpha says where it glows.
    pub self_illum: Option<[f32; 3]>,
    /// Drawn without lighting, as `UnlitGeneric` and sky materials are.
    pub unlit: bool,
}

impl Material {
    fn missing() -> Self {
        Self {
            texture: None,
            alpha: AlphaMode::Opaque,
            surface: Surface::Normal,
            self_illum: None,
            unlit: false,
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
    self_illum: Option<bool>,
    self_illum_tint: Option<[f32; 3]>,
    envmap: Option<bool>,
    keywords: Option<String>,
    surface: Option<Surface>,
    /// The shader the material uses, lower case, such as `lightmappedgeneric`.
    shader: Option<String>,
}

/// What the texture browser's filters need to know about a material.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialInfo {
    /// Blended or alpha-tested.
    pub translucent: bool,
    pub self_illum: bool,
    /// Has an `$envmap`, so it reflects.
    pub envmap: bool,
    /// The VMT's `%keywords`, for searching.
    pub keywords: String,
    /// The VMT cannot be read or its texture is missing.
    pub error: bool,
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
            self_illum: flag("$selfillum"),
            self_illum_tint: block.get("$selfillumtint").and_then(material_color),
            envmap: block.get("$envmap").map(|value| !value.trim().is_empty()),
            keywords: block.get("%keywords").map(str::to_owned),
            surface: Self::surface(block),
            shader: (!block.name.eq_ignore_ascii_case("patch")).then(|| block.name.to_ascii_lowercase()),
        }
    }

    /// What the `%compile` keys make of the material's faces, if any is set.
    fn surface(block: &Block) -> Option<Surface> {
        let set = |key: &str| {
            block
                .get(key)
                .and_then(|value| value.trim().parse::<f32>().ok())
                .is_some_and(|value| value != 0.0)
        };
        if set("%compilesky") || set("%compile2dsky") {
            Some(Surface::Sky)
        } else if set("%compilenodraw") || set("%compileblocklight") {
            Some(Surface::Nodraw)
        } else if TOOL_KEYS.iter().any(|key| set(key)) {
            Some(Surface::Tool)
        } else {
            None
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
            self_illum: other.self_illum.or(self.self_illum),
            self_illum_tint: other.self_illum_tint.or(self.self_illum_tint),
            envmap: other.envmap.or(self.envmap),
            keywords: other.keywords.or(self.keywords),
            surface: other.surface.or(self.surface),
            shader: other.shader.or(self.shader),
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
        let unlit = properties
            .shader
            .as_deref()
            .is_some_and(|shader| matches!(shader, "unlitgeneric" | "unlittwotexture" | "sky" | "sprite"));
        Material {
            texture,
            alpha: properties.alpha_mode(),
            surface: properties.surface.unwrap_or_default(),
            self_illum: properties
                .self_illum
                .unwrap_or(false)
                .then(|| properties.self_illum_tint.unwrap_or([1.0; 3])),
            unlit,
        }
    }

    /// Loads a texture by its own name, such as `effects/flashlight001`, as projected
    /// textures and skyboxes name them, at most `max_side` pixels across.
    pub fn texture_image(&self, texture: &str, max_side: u32) -> Option<Image> {
        let name = normalize(texture);
        let name = name.strip_prefix("materials/").unwrap_or(&name);
        let name = name.strip_suffix(".vtf").unwrap_or(name);
        let data = self.filesystem.read(&format!("materials/{name}.vtf"))?;
        vtf::decode(&data, max_side).ok()
    }

    /// Loads a material's texture at most `max_side` pixels across, for thumbnails.
    pub fn thumbnail(&self, material: &str, max_side: u32) -> Option<Image> {
        let key = material_key(material);
        let name = self.properties(&key, 0)?.texture?;
        let data = self.filesystem.read(&format!("materials/{name}.vtf"))?;
        vtf::decode(&data, max_side).ok()
    }

    /// Every material in the game, such as `brick/brickwall001a`: the `.vmt` files under
    /// `materials/`, sorted.
    pub fn list(&self) -> Vec<String> {
        self.list_sourced().into_iter().map(|(name, _)| name).collect()
    }

    /// Like [`MaterialLoader::list`], with every source that has each material, as
    /// [`GameFs::sources`] lists them; the first is the one it loads from.
    pub fn list_sourced(&self) -> Vec<(String, Vec<usize>)> {
        self.filesystem
            .list_sourced("materials/", ".vmt")
            .into_iter()
            .filter_map(|(path, sources)| {
                let name = path.strip_prefix("materials/")?.strip_suffix(".vmt")?;
                Some((name.to_owned(), sources))
            })
            .collect()
    }

    /// The mounted sources of the game's files.
    pub fn sources(&self) -> Vec<crate::Source> {
        self.filesystem.sources().to_vec()
    }

    /// Where a material's VMT lies on disk, when it is a loose file.
    pub fn locate(&self, material: &str) -> Option<std::path::PathBuf> {
        self.filesystem
            .locate(&format!("materials/{}.vmt", material_key(material)))
    }

    /// A material's VMT text, from a loose file or a VPK.
    pub fn source_text(&self, material: &str) -> Option<String> {
        let data = self
            .filesystem
            .read(&format!("materials/{}.vmt", material_key(material)))?;
        Some(String::from_utf8_lossy(&data).into_owned())
    }

    /// What the texture browser's filters need to know about a material.
    pub fn describe(&self, material: &str) -> MaterialInfo {
        let key = material_key(material);
        let Some(properties) = self.properties(&key, 0) else {
            return MaterialInfo {
                error: true,
                ..MaterialInfo::default()
            };
        };
        let texture_exists = properties
            .texture
            .as_ref()
            .is_some_and(|name| self.filesystem.exists(&format!("materials/{name}.vtf")));
        MaterialInfo {
            translucent: !matches!(properties.alpha_mode(), AlphaMode::Opaque),
            self_illum: properties.self_illum.unwrap_or(false),
            envmap: properties.envmap.unwrap_or(false),
            keywords: properties.keywords.unwrap_or_default(),
            error: !texture_exists,
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

/// A material colour such as `[1 0.5 0.5]` or `{255 128 128}`: square brackets hold 0 to 1
/// values, braces 0 to 255 ones.
fn material_color(text: &str) -> Option<[f32; 3]> {
    let bytes = text.trim().starts_with('{');
    let values: Vec<f32> = text
        .split(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | '{' | '}'))
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<f32>().ok())
        .collect::<Option<_>>()?;
    let scale = if bytes { 1.0 / 255.0 } else { 1.0 };
    match values.as_slice() {
        [grey] => Some([grey * scale; 3]),
        [r, g, b, ..] => Some([r * scale, g * scale, b * scale]),
        _ => None,
    }
}

/// The cache key for a material: lower case, forward slashes, without the `materials/`
/// prefix or `.vmt` extension.
fn material_key(material: &str) -> String {
    let key = normalize(material);
    let key = key.strip_prefix("materials/").unwrap_or(&key);
    key.strip_suffix(".vmt").unwrap_or(key).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_game_materials_and_makes_thumbnails() {
        let Some(dir) = std::env::var_os("MALLET_GAME_DIR") else {
            return;
        };
        let materials = Materials::new(GameFs::from_gameinfo(std::path::Path::new(&dir)).unwrap());
        let loader = materials.loader();
        let started = std::time::Instant::now();
        let list = loader.list();
        eprintln!("{} materials listed in {:?}", list.len(), started.elapsed());
        assert!(list.iter().any(|name| name == "tools/toolsnodraw"));
        let started = std::time::Instant::now();
        let thumbnail = loader.thumbnail("brick/brickwall001a", 128).unwrap();
        eprintln!(
            "thumbnail {}x{} in {:?}",
            thumbnail.pixel_width,
            thumbnail.pixel_height,
            started.elapsed()
        );
        assert!(thumbnail.pixel_width <= 128 && thumbnail.pixel_height <= 128);
    }
}

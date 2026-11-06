//! Document, entity, solid and side types.

use std::error::Error;
use std::fmt;

use glam::DVec3;
use mallet_kv::{Block, Item, Pair, ParseError};

/// Why a VMF could not be loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum VmfError {
    /// The text is not valid KeyValues.
    Syntax(ParseError),
    /// The text parsed, but a block Mallet types is malformed.
    Invalid {
        /// Which block failed, such as `side 42`.
        context: String,
        /// What is wrong with it.
        message: String,
    },
}

impl fmt::Display for VmfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(error) => write!(f, "syntax error at {error}"),
            Self::Invalid { context, message } => write!(f, "{context}: {message}"),
        }
    }
}

impl Error for VmfError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Syntax(error) => Some(error),
            Self::Invalid { .. } => None,
        }
    }
}

impl From<ParseError> for VmfError {
    fn from(error: ParseError) -> Self {
        Self::Syntax(error)
    }
}

fn invalid(context: &str, message: impl Into<String>) -> VmfError {
    VmfError::Invalid {
        context: context.to_owned(),
        message: message.into(),
    }
}

/// A whole VMF document, with top-level blocks kept in file order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Vmf {
    /// Top-level items in file order.
    pub items: Vec<TopLevel>,
}

/// One top-level item of a VMF.
#[derive(Debug, Clone, PartialEq)]
pub enum TopLevel {
    /// A stray top-level pair. Hammer never writes these, but they are preserved.
    Pair(Pair),
    /// The `world` block, holding worldspawn's keys and the world brushes.
    World(Entity),
    /// An `entity` block.
    Entity(Entity),
    /// Any other block, such as `versioninfo`, `viewsettings`, `cameras` or `cordons`.
    Other(Block),
}

impl Vmf {
    /// Parses VMF text.
    pub fn parse(text: &str) -> Result<Self, VmfError> {
        Self::from_items(mallet_kv::parse(text)?)
    }

    /// Builds a document from already-parsed KeyValues items.
    pub fn from_items(items: Vec<Item>) -> Result<Self, VmfError> {
        let items = items
            .into_iter()
            .map(|item| match item {
                Item::Pair(pair) => Ok(TopLevel::Pair(pair)),
                Item::Block(block) if block.name.eq_ignore_ascii_case("world") => {
                    Entity::from_block(block).map(TopLevel::World)
                }
                Item::Block(block) if block.name.eq_ignore_ascii_case("entity") => {
                    Entity::from_block(block).map(TopLevel::Entity)
                }
                Item::Block(block) => Ok(TopLevel::Other(block)),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { items })
    }

    /// Converts the document back into KeyValues items.
    pub fn to_items(&self) -> Vec<Item> {
        self.items
            .iter()
            .map(|item| match item {
                TopLevel::Pair(pair) => Item::Pair(pair.clone()),
                TopLevel::World(world) => Item::Block(world.to_block("world")),
                TopLevel::Entity(entity) => Item::Block(entity.to_block("entity")),
                TopLevel::Other(block) => Item::Block(block.clone()),
            })
            .collect()
    }

    /// Writes the document as VMF text in Hammer's layout.
    pub fn to_text(&self) -> String {
        mallet_kv::write(&self.to_items())
    }

    /// Returns the `world` block, if the document has one.
    pub fn world(&self) -> Option<&Entity> {
        self.items.iter().find_map(|item| match item {
            TopLevel::World(world) => Some(world),
            _ => None,
        })
    }

    /// Iterates over every entity except the world, in file order.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.items.iter().filter_map(|item| match item {
            TopLevel::Entity(entity) => Some(entity),
            _ => None,
        })
    }

    /// Iterates over the world and every entity, in file order.
    pub fn all_entities(&self) -> impl Iterator<Item = &Entity> {
        self.items.iter().filter_map(|item| match item {
            TopLevel::World(entity) | TopLevel::Entity(entity) => Some(entity),
            _ => None,
        })
    }

    /// Returns the first top-level block with the given name, such as `versioninfo`.
    pub fn block(&self, name: &str) -> Option<&Block> {
        self.items.iter().find_map(|item| match item {
            TopLevel::Other(block) if block.name.eq_ignore_ascii_case(name) => Some(block),
            _ => None,
        })
    }
}

/// The world or a single entity: its keys, brushes and editor blocks in file order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Entity {
    /// Keys, solids and other blocks in file order.
    pub items: Vec<EntityItem>,
}

/// One item inside an entity.
#[derive(Debug, Clone, PartialEq)]
pub enum EntityItem {
    /// A key, such as `classname` or `origin`.
    Key(Pair),
    /// A brush belonging to this entity.
    Solid(Solid),
    /// Any other block, such as `connections`, `editor` or `hidden`.
    Other(Block),
}

impl Entity {
    /// Builds an entity from a `world` or `entity` block.
    pub fn from_block(block: Block) -> Result<Self, VmfError> {
        let items = block
            .items
            .into_iter()
            .map(|item| match item {
                Item::Pair(pair) => Ok(EntityItem::Key(pair)),
                Item::Block(child) if child.name.eq_ignore_ascii_case("solid") => {
                    Solid::from_block(child).map(EntityItem::Solid)
                }
                Item::Block(child) => Ok(EntityItem::Other(child)),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { items })
    }

    /// Converts the entity back into a block with the given name.
    pub fn to_block(&self, name: &str) -> Block {
        Block {
            name: name.to_owned(),
            items: self
                .items
                .iter()
                .map(|item| match item {
                    EntityItem::Key(pair) => Item::Pair(pair.clone()),
                    EntityItem::Solid(solid) => Item::Block(solid.to_block()),
                    EntityItem::Other(block) => Item::Block(block.clone()),
                })
                .collect(),
        }
    }

    /// Returns the value of the first key that matches, ignoring ASCII case.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.items.iter().find_map(|item| match item {
            EntityItem::Key(pair) if pair.key.eq_ignore_ascii_case(key) => Some(pair.value.as_str()),
            _ => None,
        })
    }

    /// Sets a key, replacing the first existing value or adding it after the last key.
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        let existing = self.items.iter_mut().find_map(|item| match item {
            EntityItem::Key(pair) if pair.key.eq_ignore_ascii_case(key) => Some(pair),
            _ => None,
        });
        if let Some(pair) = existing {
            pair.value = value;
            return;
        }
        let insert_at = self
            .items
            .iter()
            .rposition(|item| matches!(item, EntityItem::Key(_)))
            .map_or(0, |index| index + 1);
        self.items
            .insert(insert_at, EntityItem::Key(Pair::new(key, value)));
    }

    /// Removes the first key that matches, ignoring ASCII case, and returns its value.
    pub fn remove(&mut self, key: &str) -> Option<String> {
        let index = self.items.iter().position(
            |item| matches!(item, EntityItem::Key(pair) if pair.key.eq_ignore_ascii_case(key)),
        )?;
        match self.items.remove(index) {
            EntityItem::Key(pair) => Some(pair.value),
            _ => None,
        }
    }

    /// The entity's unique `id`, if present and numeric.
    pub fn id(&self) -> Option<u32> {
        self.get("id")?.trim().parse().ok()
    }

    /// The entity's `classname`, or an empty string when missing.
    pub fn classname(&self) -> &str {
        self.get("classname").unwrap_or("")
    }

    /// The entity's `origin`, if present and well formed.
    pub fn origin(&self) -> Option<DVec3> {
        parse_vec3(self.get("origin")?)
    }

    /// Iterates over the brushes that belong to this entity, excluding hidden ones.
    pub fn solids(&self) -> impl Iterator<Item = &Solid> {
        self.items.iter().filter_map(|item| match item {
            EntityItem::Solid(solid) => Some(solid),
            _ => None,
        })
    }

    /// The entity's `editor` block, holding its colour, visgroups and logical position.
    pub fn editor(&self) -> Option<&Block> {
        self.items.iter().find_map(|item| match item {
            EntityItem::Other(block) if block.name.eq_ignore_ascii_case("editor") => Some(block),
            _ => None,
        })
    }

    /// The colour Hammer draws this entity with in the 2D views.
    pub fn editor_color(&self) -> Option<[u8; 3]> {
        parse_color(self.editor()?.get("color")?)
    }
}

/// A brush: a convex solid bounded by the planes of its sides.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Solid {
    /// Keys, sides and other blocks in file order.
    pub items: Vec<SolidItem>,
}

/// One item inside a solid.
#[derive(Debug, Clone, PartialEq)]
pub enum SolidItem {
    /// A key, normally just `id`.
    Key(Pair),
    /// One face of the brush.
    Side(Side),
    /// Any other block, normally `editor`.
    Other(Block),
}

impl Solid {
    /// Builds a solid from a `solid` block.
    pub fn from_block(block: Block) -> Result<Self, VmfError> {
        let items = block
            .items
            .into_iter()
            .map(|item| match item {
                Item::Pair(pair) => Ok(SolidItem::Key(pair)),
                Item::Block(child) if child.name.eq_ignore_ascii_case("side") => {
                    Side::from_block(child).map(SolidItem::Side)
                }
                Item::Block(child) => Ok(SolidItem::Other(child)),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { items })
    }

    /// Converts the solid back into a `solid` block.
    pub fn to_block(&self) -> Block {
        Block {
            name: "solid".to_owned(),
            items: self
                .items
                .iter()
                .map(|item| match item {
                    SolidItem::Key(pair) => Item::Pair(pair.clone()),
                    SolidItem::Side(side) => Item::Block(side.to_block()),
                    SolidItem::Other(block) => Item::Block(block.clone()),
                })
                .collect(),
        }
    }

    /// The solid's unique `id`, if present and numeric.
    pub fn id(&self) -> Option<u32> {
        self.items.iter().find_map(|item| match item {
            SolidItem::Key(pair) if pair.key.eq_ignore_ascii_case("id") => pair.value.trim().parse().ok(),
            _ => None,
        })
    }

    /// Iterates over the solid's faces.
    pub fn sides(&self) -> impl Iterator<Item = &Side> {
        self.items.iter().filter_map(|item| match item {
            SolidItem::Side(side) => Some(side),
            _ => None,
        })
    }

    /// The solid's `editor` block.
    pub fn editor(&self) -> Option<&Block> {
        self.items.iter().find_map(|item| match item {
            SolidItem::Other(block) if block.name.eq_ignore_ascii_case("editor") => Some(block),
            _ => None,
        })
    }

    /// The colour Hammer draws this brush with in the 2D views.
    pub fn editor_color(&self) -> Option<[u8; 3]> {
        parse_color(self.editor()?.get("color")?)
    }
}

/// A texture projection axis, written as `[x y z offset] scale`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexAxis {
    /// World-space direction the texture runs along.
    pub axis: DVec3,
    /// Texture shift in texels.
    pub offset: f64,
    /// World units per texel.
    pub scale: f64,
}

impl TexAxis {
    /// Parses `[x y z offset] scale`.
    pub fn parse(text: &str) -> Option<Self> {
        let [x, y, z, offset, scale] = parse_numbers(text)?.try_into().ok()?;
        Some(Self {
            axis: DVec3::new(x, y, z),
            offset,
            scale,
        })
    }
}

impl fmt::Display for TexAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{} {} {} {}] {}",
            self.axis.x, self.axis.y, self.axis.z, self.offset, self.scale
        )
    }
}

/// One face of a brush.
///
/// The fields Hammer always writes are typed. Anything else, such as a `dispinfo` block,
/// is kept verbatim in [`Side::extra`] and written after the typed fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Side {
    /// Unique face id.
    pub id: u32,
    /// Three points on the face plane, wound so the normal points out of the brush.
    pub plane: [DVec3; 3],
    /// Material path relative to `materials/`, without extension.
    pub material: String,
    /// Horizontal texture axis.
    pub uaxis: TexAxis,
    /// Vertical texture axis.
    pub vaxis: TexAxis,
    /// Texture rotation in degrees, as shown in the face edit sheet.
    pub rotation: Option<f64>,
    /// Luxels per world unit used by VRAD.
    pub lightmap_scale: Option<u32>,
    /// Bitmask of smoothing groups.
    pub smoothing_groups: Option<u32>,
    /// Every other key and block, in file order.
    pub extra: Vec<Item>,
}

impl Side {
    /// Builds a side from a `side` block.
    pub fn from_block(block: Block) -> Result<Self, VmfError> {
        let context = match block.get("id") {
            Some(id) => format!("side {id}"),
            None => "side".to_owned(),
        };
        let mut id = None;
        let mut plane = None;
        let mut material = None;
        let mut uaxis = None;
        let mut vaxis = None;
        let mut rotation = None;
        let mut lightmap_scale = None;
        let mut smoothing_groups = None;
        let mut extra = Vec::new();

        for item in block.items {
            let Item::Pair(pair) = item else {
                extra.push(item);
                continue;
            };
            match pair.key.to_ascii_lowercase().as_str() {
                "id" if id.is_none() => id = Some(parse_value(&context, &pair)?),
                "plane" if plane.is_none() => {
                    plane = Some(parse_plane(&pair.value).ok_or_else(|| bad_value(&context, &pair))?)
                }
                "material" if material.is_none() => material = Some(pair.value),
                "uaxis" if uaxis.is_none() => {
                    uaxis = Some(TexAxis::parse(&pair.value).ok_or_else(|| bad_value(&context, &pair))?)
                }
                "vaxis" if vaxis.is_none() => {
                    vaxis = Some(TexAxis::parse(&pair.value).ok_or_else(|| bad_value(&context, &pair))?)
                }
                "rotation" if rotation.is_none() => rotation = Some(parse_value(&context, &pair)?),
                "lightmapscale" if lightmap_scale.is_none() => {
                    lightmap_scale = Some(parse_value(&context, &pair)?)
                }
                "smoothing_groups" if smoothing_groups.is_none() => {
                    smoothing_groups = Some(parse_value(&context, &pair)?)
                }
                _ => extra.push(Item::Pair(pair)),
            }
        }

        Ok(Self {
            id: id.ok_or_else(|| invalid(&context, "missing \"id\""))?,
            plane: plane.ok_or_else(|| invalid(&context, "missing \"plane\""))?,
            material: material.ok_or_else(|| invalid(&context, "missing \"material\""))?,
            uaxis: uaxis.ok_or_else(|| invalid(&context, "missing \"uaxis\""))?,
            vaxis: vaxis.ok_or_else(|| invalid(&context, "missing \"vaxis\""))?,
            rotation,
            lightmap_scale,
            smoothing_groups,
            extra,
        })
    }

    /// Converts the side back into a `side` block, typed fields first in Hammer's order.
    pub fn to_block(&self) -> Block {
        let mut block = Block::new("side");
        block.push_pair("id", self.id.to_string());
        block.push_pair("plane", format_plane(&self.plane));
        block.push_pair("material", self.material.clone());
        block.push_pair("uaxis", self.uaxis.to_string());
        block.push_pair("vaxis", self.vaxis.to_string());
        if let Some(rotation) = self.rotation {
            block.push_pair("rotation", rotation.to_string());
        }
        if let Some(scale) = self.lightmap_scale {
            block.push_pair("lightmapscale", scale.to_string());
        }
        if let Some(groups) = self.smoothing_groups {
            block.push_pair("smoothing_groups", groups.to_string());
        }
        block.items.extend(self.extra.iter().cloned());
        block
    }

    /// The face's displacement data, if it is a displacement.
    pub fn dispinfo(&self) -> Option<&Block> {
        self.extra.iter().find_map(|item| match item {
            Item::Block(block) if block.name.eq_ignore_ascii_case("dispinfo") => Some(block),
            _ => None,
        })
    }
}

fn bad_value(context: &str, pair: &Pair) -> VmfError {
    invalid(context, format!("bad \"{}\" value \"{}\"", pair.key, pair.value))
}

fn parse_value<T: std::str::FromStr>(context: &str, pair: &Pair) -> Result<T, VmfError> {
    pair.value.trim().parse().map_err(|_| bad_value(context, pair))
}

/// Splits text such as `(1 2 3) (4 5 6)` or `[1 0 0 0] 0.25` into its numbers.
///
/// Returns `None` if any token is not a number.
pub fn parse_numbers(text: &str) -> Option<Vec<f64>> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']'))
        .filter(|token| !token.is_empty())
        .map(|token| token.parse().ok())
        .collect()
}

/// Parses three whitespace-separated numbers, with or without parentheses.
pub fn parse_vec3(text: &str) -> Option<DVec3> {
    let [x, y, z] = parse_numbers(text)?.try_into().ok()?;
    Some(DVec3::new(x, y, z))
}

/// Parses an `R G B` colour with components from 0 to 255.
pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    let mut parts = text.split_whitespace().map(|part| part.parse::<u8>().ok());
    let color = [parts.next()??, parts.next()??, parts.next()??];
    Some(color)
}

/// Parses a side's `(x y z) (x y z) (x y z)` plane.
pub fn parse_plane(text: &str) -> Option<[DVec3; 3]> {
    let numbers = parse_numbers(text)?;
    let [ax, ay, az, bx, by, bz, cx, cy, cz] = numbers.try_into().ok()?;
    Some([
        DVec3::new(ax, ay, az),
        DVec3::new(bx, by, bz),
        DVec3::new(cx, cy, cz),
    ])
}

/// Formats a plane the way Hammer writes it.
pub fn format_plane(plane: &[DVec3; 3]) -> String {
    let [a, b, c] = plane;
    format!(
        "({} {} {}) ({} {} {}) ({} {} {})",
        a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z
    )
}

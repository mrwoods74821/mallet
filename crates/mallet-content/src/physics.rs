//! Collision models (`.phy` files) that sit beside studio models.
//!
//! A `.phy` file holds one solid per physics bone. Each solid is an IVP compact surface:
//! a tree whose leaves are convex pieces ("ledges"), stored as triangles over a point
//! array in metres, in the physics engine's axes.

use crate::vpk::normalize;
use crate::GameFs;

/// Inches per metre, the scale between the physics engine and world units.
const INCHES_PER_METRE: f32 = 1.0 / 0.0254;

/// Size of the `.phy` file header.
const FILE_HEADER_SIZE: usize = 16;

/// Size of the header in front of each solid's compact surface in newer files.
const SOLID_HEADER_SIZE: usize = 28;

/// Size of an IVP compact surface header.
const SURFACE_HEADER_SIZE: usize = 48;

/// Size of one node of the ledge tree.
const TREE_NODE_SIZE: usize = 28;

/// Size of a compact ledge header, and of each of its triangles.
const LEDGE_HEADER_SIZE: usize = 16;
const TRIANGLE_SIZE: usize = 16;

/// Size of one stored point: three floats and padding.
const POINT_SIZE: usize = 16;

/// Deepest ledge tree accepted, so a corrupt file cannot recurse forever.
const MAX_TREE_DEPTH: usize = 64;

/// A model's collision geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct CollisionModel {
    /// One entry per solid, in file order; ragdolls have one solid per physics bone.
    pub solids: Vec<CollisionSolid>,
}

/// One rigid part of a collision model.
#[derive(Debug, Clone, PartialEq)]
pub struct CollisionSolid {
    /// Convex pieces, each given by its corner points in model units (X forward, Y left,
    /// Z up), relative to the solid's bone.
    pub pieces: Vec<Vec<[f32; 3]>>,
}

impl CollisionModel {
    /// Number of convex pieces in all solids.
    pub fn piece_count(&self) -> usize {
        self.solids.iter().map(|solid| solid.pieces.len()).sum()
    }
}

/// Loads the collision model of a studio model such as `models/props_c17/fence01a.mdl`.
///
/// Fails when the model has no `.phy` file, as purely visual props do.
pub fn load_collision(filesystem: &GameFs, path: &str) -> Result<CollisionModel, String> {
    let path = normalize(path);
    let base = path.strip_suffix(".mdl").unwrap_or(&path);
    let file = format!("{base}.phy");
    let data = filesystem
        .read(&file)
        .ok_or_else(|| format!("{base}.mdl has no collision model"))?;
    parse_collision(&data).map_err(|error| format!("{file}: {error}"))
}

/// Parses the contents of a `.phy` file.
pub fn parse_collision(data: &[u8]) -> Result<CollisionModel, String> {
    let reader = Reader(data);
    let header_size = reader.i32(0)? as usize;
    let solid_count = reader.i32(8)?;
    if header_size < FILE_HEADER_SIZE || !(0..=1024).contains(&solid_count) {
        return Err("not a collision model".to_owned());
    }
    let mut solids = Vec::new();
    let mut offset = header_size;
    for _ in 0..solid_count {
        let size = reader.i32(offset)?;
        let start = offset + 4;
        if size <= 0 || start + size as usize > data.len() {
            return Err("a solid runs past the end of the file".to_owned());
        }
        let surface = if data.get(start..start + 4) == Some(b"VPHY") {
            start + SOLID_HEADER_SIZE
        } else {
            start
        };
        solids.push(read_surface(&reader, surface)?);
        offset = start + size as usize;
    }
    Ok(CollisionModel { solids })
}

/// Reads the convex pieces of one compact surface by walking its ledge tree.
fn read_surface(reader: &Reader, surface: usize) -> Result<CollisionSolid, String> {
    let root = reader.i32(surface + 32)?;
    if root < SURFACE_HEADER_SIZE as i32 {
        return Err("a solid has no ledge tree".to_owned());
    }
    let mut leaves = Vec::new();
    collect_leaves(reader, surface + root as usize, 0, &mut leaves)?;
    let pieces = leaves
        .into_iter()
        .map(|ledge| read_ledge(reader, ledge))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|points| points.len() >= 4)
        .collect();
    Ok(CollisionSolid { pieces })
}

/// Collects the ledges at the leaves of the tree below `node`. Inner nodes also carry a
/// ledge, the hull of everything under them, which is skipped.
fn collect_leaves(reader: &Reader, node: usize, depth: usize, leaves: &mut Vec<usize>) -> Result<(), String> {
    if depth > MAX_TREE_DEPTH {
        return Err("the ledge tree is too deep".to_owned());
    }
    let right = reader.i32(node)?;
    if right == 0 {
        let ledge = reader.i32(node + 4)?;
        leaves.push(offset_by(node, ledge)?);
        return Ok(());
    }
    collect_leaves(reader, node + TREE_NODE_SIZE, depth + 1, leaves)?;
    collect_leaves(reader, offset_by(node, right)?, depth + 1, leaves)
}

/// The distinct corner points of one ledge, converted to model units and axes.
fn read_ledge(reader: &Reader, ledge: usize) -> Result<Vec<[f32; 3]>, String> {
    let points = offset_by(ledge, reader.i32(ledge)?)?;
    let triangles = reader.i16(ledge + 12)?.max(0) as usize;
    let mut indices = Vec::with_capacity(triangles * 3);
    for triangle in 0..triangles {
        let at = ledge + LEDGE_HEADER_SIZE + triangle * TRIANGLE_SIZE;
        for edge in 0..3 {
            indices.push((reader.u32(at + 4 + edge * 4)? & 0xFFFF) as usize);
        }
    }
    indices.sort_unstable();
    indices.dedup();
    indices
        .into_iter()
        .map(|index| {
            let at = points + index * POINT_SIZE;
            let [x, y, z] = [reader.f32(at)?, reader.f32(at + 4)?, reader.f32(at + 8)?];
            Ok([x, z, -y].map(|value| value * INCHES_PER_METRE))
        })
        .collect()
}

/// `base` moved by a signed offset, failing when that leaves the file's address range.
fn offset_by(base: usize, offset: i32) -> Result<usize, String> {
    base.checked_add_signed(offset as isize)
        .ok_or_else(|| "an offset points outside the file".to_owned())
}

/// Little-endian reads that fail instead of panicking past the end of the data.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn bytes<const N: usize>(&self, at: usize) -> Result<[u8; N], String> {
        self.0
            .get(at..at + N)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| "the file is cut short".to_owned())
    }

    fn i32(&self, at: usize) -> Result<i32, String> {
        self.bytes(at).map(i32::from_le_bytes)
    }

    fn u32(&self, at: usize) -> Result<u32, String> {
        self.bytes(at).map(u32::from_le_bytes)
    }

    fn i16(&self, at: usize) -> Result<i16, String> {
        self.bytes(at).map(i16::from_le_bytes)
    }

    fn f32(&self, at: usize) -> Result<f32, String> {
        self.bytes(at).map(f32::from_le_bytes)
    }
}

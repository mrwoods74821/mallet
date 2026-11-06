//! Studio models (`.mdl` with its `.vvd` and `.vtx`), as placed by prop entities.

use vmdl::{Mdl, Vtx, Vvd};

use crate::vpk::normalize;
use crate::GameFs;

/// One vertex of a model, in the model's own space (X forward, Y left, Z up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// The triangles of a model that share one material.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelMesh {
    /// Material path under `materials/`, without extension, such as
    /// `models/props_c17/oildrum001`.
    pub material: String,
    /// Triangle list indices into [`Model::vertices`].
    pub indices: Vec<u32>,
}

/// A model's highest-detail geometry with its default skin.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub vertices: Vec<ModelVertex>,
    pub meshes: Vec<ModelMesh>,
    /// Smallest corner of the geometry.
    pub min: [f32; 3],
    /// Largest corner of the geometry.
    pub max: [f32; 3],
}

impl Model {
    /// Number of triangles in all meshes.
    pub fn triangle_count(&self) -> usize {
        self.meshes.iter().map(|mesh| mesh.indices.len() / 3).sum()
    }
}

/// Loads a model such as `models/props_c17/oildrum001.mdl` from the game's files.
///
/// Uses the first level of detail, the default skin and the model's idle orientation.
/// Each mesh's material is looked up in the model's material folders in order, like the
/// engine does.
pub fn load_model(filesystem: &GameFs, path: &str) -> Result<Model, String> {
    // The parser panics on some model features it does not support yet, such as
    // animations stored in a separate block, so a panic counts as a failed load.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read_model(filesystem, path)))
        .unwrap_or_else(|_| Err(format!("{path} uses features the model parser does not support")))
}

fn read_model(filesystem: &GameFs, path: &str) -> Result<Model, String> {
    let path = normalize(path);
    let base = path.strip_suffix(".mdl").unwrap_or(&path);
    let read = |suffix: &str| {
        filesystem
            .read(&format!("{base}{suffix}"))
            .ok_or_else(|| format!("{base}{suffix} not found"))
    };
    let mdl = Mdl::read(&read(".mdl")?).map_err(|error| error.to_string())?;
    let vtx_data = read(".dx90.vtx").or_else(|_| read(".vtx"))?;
    let vtx = Vtx::read(&vtx_data).map_err(|error| error.to_string())?;
    let vvd = Vvd::read(&read(".vvd")?).map_err(|error| error.to_string())?;
    let model = vmdl::Model::from_parts(mdl, vtx, vvd);

    let vertices: Vec<ModelVertex> = model
        .vertices()
        .iter()
        .map(|vertex| {
            let position = model.apply_root_transform(vertex.position);
            let normal = model.apply_root_transform(vertex.normal);
            ModelVertex {
                position: [position.x, position.y, position.z],
                normal: [normal.x, normal.y, normal.z],
                uv: vertex.texture_coordinates,
            }
        })
        .collect();

    let directories: Vec<String> = model
        .texture_directories()
        .iter()
        .map(|directory| normalize(directory).trim_end_matches('/').to_owned())
        .collect();
    let skin = model.skin_tables().next();
    let mut meshes = Vec::new();
    for mesh in model.meshes() {
        let index = mesh.material_index();
        let texture = skin
            .as_ref()
            .and_then(|skin| skin.texture(index))
            .or_else(|| model.textures().get(index as usize).map(|info| info.name.as_str()))
            .unwrap_or_default();
        let indices: Vec<u32> = mesh
            .vertex_strip_indices()
            .flatten()
            .filter(|index| *index < vertices.len())
            .map(|index| index as u32)
            .collect();
        if indices.len() < 3 {
            continue;
        }
        meshes.push(ModelMesh {
            material: resolve_material(filesystem, &directories, texture),
            indices,
        });
    }

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for vertex in &vertices {
        for axis in 0..3 {
            min[axis] = min[axis].min(vertex.position[axis]);
            max[axis] = max[axis].max(vertex.position[axis]);
        }
    }
    if vertices.is_empty() {
        return Err(format!("{base}.mdl has no vertices"));
    }
    Ok(Model {
        vertices,
        meshes,
        min,
        max,
    })
}

/// The first `directory/texture` that has a material file, or the first candidate when
/// none does, so the renderer shows its missing-texture pattern.
fn resolve_material(filesystem: &GameFs, directories: &[String], texture: &str) -> String {
    let texture = normalize(texture);
    let candidates: Vec<String> = if directories.is_empty() {
        vec![texture.clone()]
    } else {
        directories
            .iter()
            .map(|directory| {
                if directory.is_empty() {
                    texture.clone()
                } else {
                    format!("{directory}/{texture}")
                }
            })
            .collect()
    };
    candidates
        .iter()
        .find(|candidate| filesystem.exists(&format!("materials/{candidate}.vmt")))
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

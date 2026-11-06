//! Wireframe geometry prepared once per map change and shared by all four viewports.

use std::collections::HashSet;

use eframe::egui::Color32;
use glam::DVec3;
use mallet_vmf::geometry::{faces_bounds, solid_faces, Bounds};
use mallet_vmf::{Solid, Vmf};

/// Colour used when a brush or entity has no editor colour.
const DEFAULT_COLOR: Color32 = Color32::from_rgb(220, 220, 220);

/// Half-size of the box drawn for point entities until FGD sizes are loaded.
const POINT_ENTITY_HALF_SIZE: f64 = 8.0;

/// Edge coordinates are rounded to this many steps per unit when removing shared edges.
const EDGE_QUANTIZE: f64 = 100.0;

/// The edges of one brush or point entity.
pub struct WireMesh {
    pub color: Color32,
    pub edges: Vec<[DVec3; 2]>,
    pub bounds: Bounds,
}

/// Everything the viewports draw.
#[derive(Default)]
pub struct Scene {
    pub meshes: Vec<WireMesh>,
    pub bounds: Bounds,
    pub solid_count: usize,
    pub entity_count: usize,
}

impl Scene {
    /// Builds wireframes for every visible brush and point entity in the map.
    ///
    /// World brushes use their own editor colour; brushes of brush entities use the
    /// entity's colour, matching Hammer's 2D views.
    pub fn build(vmf: &Vmf) -> Self {
        let mut scene = Self::default();
        for entity in vmf.all_entities() {
            let is_world = entity.classname().eq_ignore_ascii_case("worldspawn");
            if !is_world {
                scene.entity_count += 1;
            }
            let entity_color = entity.editor_color();
            let mut has_solids = false;
            for solid in entity.solids() {
                has_solids = true;
                scene.solid_count += 1;
                let color = if is_world {
                    solid.editor_color()
                } else {
                    entity_color.or_else(|| solid.editor_color())
                };
                if let Some(mesh) = brush_mesh(solid, to_color(color)) {
                    scene.bounds.add_bounds(&mesh.bounds);
                    scene.meshes.push(mesh);
                }
            }
            if has_solids || is_world {
                continue;
            }
            if let Some(origin) = entity.origin() {
                let mesh = box_mesh(origin, POINT_ENTITY_HALF_SIZE, to_color(entity_color));
                scene.bounds.add_bounds(&mesh.bounds);
                scene.meshes.push(mesh);
            }
        }
        scene
    }
}

fn to_color(color: Option<[u8; 3]>) -> Color32 {
    color.map_or(DEFAULT_COLOR, |[r, g, b]| Color32::from_rgb(r, g, b))
}

fn quantize(point: DVec3) -> [i64; 3] {
    (point * EDGE_QUANTIZE).round().as_i64vec3().to_array()
}

fn brush_mesh(solid: &Solid, color: Color32) -> Option<WireMesh> {
    let faces = solid_faces(solid);
    if faces.is_empty() {
        return None;
    }
    let mut seen = HashSet::new();
    let mut edges = Vec::new();
    for face in &faces {
        let count = face.vertices.len();
        for index in 0..count {
            let a = face.vertices[index];
            let b = face.vertices[(index + 1) % count];
            let (qa, qb) = (quantize(a), quantize(b));
            let key = if qa <= qb { (qa, qb) } else { (qb, qa) };
            if seen.insert(key) {
                edges.push([a, b]);
            }
        }
    }
    Some(WireMesh {
        color,
        edges,
        bounds: faces_bounds(&faces),
    })
}

fn box_mesh(center: DVec3, half: f64, color: Color32) -> WireMesh {
    let min = center - DVec3::splat(half);
    let max = center + DVec3::splat(half);
    let corner = |x: bool, y: bool, z: bool| {
        DVec3::new(
            if x { max.x } else { min.x },
            if y { max.y } else { min.y },
            if z { max.z } else { min.z },
        )
    };
    let mut edges = Vec::with_capacity(12);
    for a in [false, true] {
        for b in [false, true] {
            edges.push([corner(false, a, b), corner(true, a, b)]);
            edges.push([corner(a, false, b), corner(a, true, b)]);
            edges.push([corner(a, b, false), corner(a, b, true)]);
        }
    }
    WireMesh {
        color,
        edges,
        bounds: Bounds { min, max },
    }
}

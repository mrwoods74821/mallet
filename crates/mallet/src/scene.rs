//! Wireframe geometry prepared once per map change and shared by all four viewports,
//! plus hit testing for selection.

use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui::Color32;
use glam::{DVec2, DVec3};
use mallet_vmf::geometry::{faces_bounds, solid_faces, Bounds, Plane};
use mallet_vmf::{Side, Solid, TexAxis, Vmf};

use crate::view2d::Axis2D;

/// Colour used when a brush or entity has no editor colour.
const DEFAULT_COLOR: Color32 = Color32::from_rgb(220, 220, 220);

/// Half-size of the box drawn for point entities until FGD sizes are loaded.
const POINT_ENTITY_HALF_SIZE: f64 = 8.0;

/// Edge coordinates are rounded to this many steps per unit when removing shared edges.
const EDGE_QUANTIZE: f64 = 100.0;

/// What a mesh belongs to, and so what clicking it selects.
///
/// World brushes are selected one by one. Any part of an entity selects the whole entity,
/// like Hammer's default "groups" selection mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Owner {
    /// A world brush, by solid id.
    Solid(u32),
    /// A point or brush entity, by entity id.
    Entity(u32),
}

/// Colour of selected brushes and entities, as in Hammer.
pub const SELECTED_COLOR: Color32 = Color32::from_rgb(255, 0, 0);
/// Colour of the block tool's pending box and the selection marquee.
pub const TOOL_COLOR: Color32 = Color32::from_rgb(255, 255, 255);

/// Colour of the box and handles drawn around the selection or the pending block.
pub const HANDLE_COLOR: Color32 = Color32::from_rgb(255, 255, 255);
/// Half the side length, in pixels, of a selection handle.
pub const HANDLE_HALF_SIZE: f32 = 3.5;

/// A per-axis stretch and shift that maps the box `from` onto the box `to`; a plain move
/// when both have the same size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remap {
    pub from: Bounds,
    pub to: Bounds,
}

impl Remap {
    /// Moves `bounds` by `offset` without resizing.
    pub fn translate(bounds: Bounds, offset: DVec3) -> Self {
        Self {
            from: bounds,
            to: Bounds {
                min: bounds.min + offset,
                max: bounds.max + offset,
            },
        }
    }

    pub fn apply(&self, point: DVec3) -> DVec3 {
        self.from.remap(&self.to, point)
    }
}

/// Temporary shapes drawn on top of the map while a tool is in use.
#[derive(Clone, Copy, Debug, Default)]
pub struct Overlay {
    /// Where selected meshes are drawn while being moved or resized.
    pub transform: Option<Remap>,
    /// The block tool's box, waiting for Enter.
    pub block: Option<Bounds>,
    /// A selection rectangle being dragged in one 2D view, as two world corners.
    pub marquee: Option<(Axis2D, DVec2, DVec2)>,
    /// The box that gets resize handles in the 2D views: the selection or the pending block.
    pub handles: Option<Bounds>,
}

impl WireMesh {
    /// Colour to draw this mesh with and where to move it, given the current overlay.
    pub fn appearance(&self, overlay: &Overlay) -> (Color32, Option<Remap>) {
        if self.selected {
            (SELECTED_COLOR, overlay.transform)
        } else {
            (self.color, None)
        }
    }

    /// The mesh bounds after an optional remap.
    pub fn mapped_bounds(&self, remap: Option<&Remap>) -> Bounds {
        match remap {
            Some(remap) => Bounds {
                min: remap.apply(self.bounds.min),
                max: remap.apply(self.bounds.max),
            },
            None => self.bounds,
        }
    }
}

/// One polygon of a brush or point entity box, for the textured 3D view.
pub struct SceneFace {
    /// The face's material, or `None` for point entity boxes drawn in a flat colour.
    pub material: Option<String>,
    /// Convex polygon in winding order.
    pub vertices: Vec<DVec3>,
    /// Outward unit normal.
    pub normal: DVec3,
    pub uaxis: TexAxis,
    pub vaxis: TexAxis,
}

/// The geometry of one brush or point entity.
pub struct WireMesh {
    pub color: Color32,
    pub edges: Vec<[DVec3; 2]>,
    /// Polygons for the textured 3D view.
    pub faces: Vec<SceneFace>,
    pub bounds: Bounds,
    /// Planes enclosing the mesh, used for 3D picking.
    pub planes: Vec<Plane>,
    /// `None` when the brush or entity has no id and cannot be selected.
    pub owner: Option<Owner>,
    pub selected: bool,
}

/// Everything the viewports draw.
#[derive(Default)]
pub struct Scene {
    pub meshes: Vec<WireMesh>,
    /// Changes whenever the geometry changes, so renderers know when to rebuild cached
    /// buffers.
    pub geometry_generation: u64,
    /// Changes whenever the selection changes.
    pub selection_generation: u64,
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
        let mut scene = Self {
            geometry_generation: next_generation(),
            selection_generation: next_generation(),
            ..Self::default()
        };
        for entity in vmf.all_entities() {
            let is_world = entity.classname().eq_ignore_ascii_case("worldspawn");
            if !is_world {
                scene.entity_count += 1;
            }
            let entity_color = entity.editor_color();
            let entity_owner = entity.id().map(Owner::Entity);
            let mut has_solids = false;
            for solid in entity.solids() {
                has_solids = true;
                scene.solid_count += 1;
                let (color, owner) = if is_world {
                    (solid.editor_color(), solid.id().map(Owner::Solid))
                } else {
                    (entity_color.or_else(|| solid.editor_color()), entity_owner)
                };
                if let Some(mesh) = brush_mesh(solid, to_color(color), owner) {
                    scene.bounds.add_bounds(&mesh.bounds);
                    scene.meshes.push(mesh);
                }
            }
            if has_solids || is_world {
                continue;
            }
            if let Some(origin) = entity.origin() {
                let half = DVec3::splat(POINT_ENTITY_HALF_SIZE);
                let bounds = Bounds {
                    min: origin - half,
                    max: origin + half,
                };
                let mesh = WireMesh {
                    color: to_color(entity_color),
                    edges: box_edges(&bounds),
                    faces: box_faces(&bounds),
                    bounds,
                    planes: box_planes(&bounds),
                    owner: entity_owner,
                    selected: false,
                };
                scene.bounds.add_bounds(&mesh.bounds);
                scene.meshes.push(mesh);
            }
        }
        scene
    }

    /// Whether any mesh belongs to `owner`.
    pub fn contains(&self, owner: Owner) -> bool {
        self.meshes.iter().any(|mesh| mesh.owner == Some(owner))
    }

    /// Flags the meshes whose owner is in the selection.
    pub fn mark_selected(&mut self, selection: &BTreeSet<Owner>) {
        for mesh in &mut self.meshes {
            mesh.selected = mesh.owner.is_some_and(|owner| selection.contains(&owner));
        }
        self.selection_generation = next_generation();
    }

    /// Bounds of every selected mesh; empty when nothing is selected.
    pub fn selection_bounds(&self) -> Bounds {
        let mut bounds = Bounds::EMPTY;
        for mesh in self.meshes.iter().filter(|mesh| mesh.selected) {
            bounds.add_bounds(&mesh.bounds);
        }
        bounds
    }

    /// Every selectable owner, without duplicates.
    pub fn owners(&self) -> BTreeSet<Owner> {
        self.meshes.iter().filter_map(|mesh| mesh.owner).collect()
    }

    /// Picks what is under a 2D view point the way Hammer does: a brush's centre cross
    /// first, then its outline. Clicking empty space inside a brush picks nothing, so a
    /// selection rectangle can start there. Among several hits the smallest brush wins.
    /// `tolerance` is how close, in world units, the point must be.
    pub fn pick_2d(&self, axis: Axis2D, point: DVec2, tolerance: f64) -> Option<Owner> {
        let smallest = |hits: &mut dyn Iterator<Item = &WireMesh>| {
            hits.filter_map(|mesh| Some((mesh.owner?, projected_area(axis, &mesh.bounds))))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(owner, _)| owner)
        };
        let on_centre = |mesh: &&WireMesh| (axis.project(mesh.bounds.center()) - point).abs().max_element() <= tolerance;
        let on_outline = |mesh: &&WireMesh| {
            let min = axis.project(mesh.bounds.min) - DVec2::splat(tolerance);
            let max = axis.project(mesh.bounds.max) + DVec2::splat(tolerance);
            point.cmpge(min).all()
                && point.cmple(max).all()
                && mesh.edges.iter().any(|[a, b]| {
                    segment_distance(point, axis.project(*a), axis.project(*b)) <= tolerance
                })
        };
        smallest(&mut self.meshes.iter().filter(on_centre)).or_else(|| smallest(&mut self.meshes.iter().filter(on_outline)))
    }

    /// Picks the nearest mesh hit by a ray.
    pub fn pick_ray(&self, origin: DVec3, direction: DVec3) -> Option<Owner> {
        self.meshes
            .iter()
            .filter_map(|mesh| Some((mesh.owner?, ray_convex(origin, direction, &mesh.planes)?)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(owner, _)| owner)
    }

    /// Owners with a centre cross inside a 2D rectangle, the way Hammer's selection
    /// rectangle picks brushes. For an entity made of several brushes, any one of their
    /// centres counts.
    pub fn owners_inside(&self, axis: Axis2D, corner_a: DVec2, corner_b: DVec2) -> BTreeSet<Owner> {
        let (min, max) = (corner_a.min(corner_b), corner_a.max(corner_b));
        self.meshes
            .iter()
            .filter(|mesh| {
                let centre = axis.project(mesh.bounds.center());
                centre.cmpge(min).all() && centre.cmple(max).all()
            })
            .filter_map(|mesh| mesh.owner)
            .collect()
    }
}

fn projected_area(axis: Axis2D, bounds: &Bounds) -> f64 {
    let size = axis.project(bounds.size());
    size.x * size.y
}

/// Shortest distance from a point to a line segment.
fn segment_distance(point: DVec2, a: DVec2, b: DVec2) -> f64 {
    let along = b - a;
    let length = along.length_squared();
    let t = if length > 0.0 { ((point - a).dot(along) / length).clamp(0.0, 1.0) } else { 0.0 };
    point.distance(a + along * t)
}

/// Distance along the ray to where it enters a convex volume, or `None` on a miss.
/// A ray starting inside the volume misses it, so clicking from inside a brush picks
/// what is beyond it, as faces seen from inside are not drawn either.
fn ray_convex(origin: DVec3, direction: DVec3, planes: &[Plane]) -> Option<f64> {
    if planes.is_empty() || planes.iter().all(|plane| plane.distance(origin) <= 0.0) {
        return None;
    }
    let mut enter: f64 = 0.0;
    let mut exit = f64::INFINITY;
    for plane in planes {
        let facing = plane.normal.dot(direction);
        let distance = plane.distance(origin);
        if facing.abs() < 1e-12 {
            if distance > 0.0 {
                return None;
            }
            continue;
        }
        let t = -distance / facing;
        if facing < 0.0 {
            enter = enter.max(t);
        } else {
            exit = exit.min(t);
        }
        if enter > exit {
            return None;
        }
    }
    Some(enter)
}

fn to_color(color: Option<[u8; 3]>) -> Color32 {
    color.map_or(DEFAULT_COLOR, |[r, g, b]| Color32::from_rgb(r, g, b))
}

fn quantize(point: DVec3) -> [i64; 3] {
    (point * EDGE_QUANTIZE).round().as_i64vec3().to_array()
}

fn brush_mesh(solid: &Solid, color: Color32, owner: Option<Owner>) -> Option<WireMesh> {
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
    let sides: Vec<&Side> = solid.sides().collect();
    let scene_faces = faces
        .iter()
        .filter_map(|face| {
            let side = sides.get(face.side)?;
            Some(SceneFace {
                material: Some(side.material.clone()),
                vertices: face.vertices.clone(),
                normal: face.plane.normal,
                uaxis: side.uaxis,
                vaxis: side.vaxis,
            })
        })
        .collect();
    Some(WireMesh {
        color,
        edges,
        faces: scene_faces,
        bounds: faces_bounds(&faces),
        planes: faces.iter().map(|face| face.plane).collect(),
        owner,
        selected: false,
    })
}

/// The 12 edges of an axis-aligned box.
pub fn box_edges(bounds: &Bounds) -> Vec<[DVec3; 2]> {
    let (min, max) = (bounds.min, bounds.max);
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
    edges
}

/// The six outward-facing quads of an axis-aligned box, without a material.
fn box_faces(bounds: &Bounds) -> Vec<SceneFace> {
    let flat_axis = TexAxis {
        axis: DVec3::ZERO,
        offset: 0.0,
        scale: 1.0,
    };
    box_planes(bounds)
        .into_iter()
        .map(|plane| {
            let normal = plane.normal;
            let (u, v) = if normal.z.abs() > 0.5 { (DVec3::X, DVec3::Y) } else { (DVec3::Z, normal.cross(DVec3::Z)) };
            let center = bounds.center() + normal * (bounds.size() * 0.5).dot(normal.abs());
            let half = bounds.size() * 0.5;
            let (du, dv) = (u * half.dot(u.abs()), v * half.dot(v.abs()));
            let mut vertices = vec![center - du - dv, center + du - dv, center + du + dv, center - du + dv];
            if (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]).dot(normal) < 0.0 {
                vertices.reverse();
            }
            SceneFace {
                material: None,
                vertices,
                normal,
                uaxis: flat_axis,
                vaxis: flat_axis,
            }
        })
        .collect()
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed)
}

fn box_planes(bounds: &Bounds) -> Vec<Plane> {
    [DVec3::X, DVec3::Y, DVec3::Z]
        .into_iter()
        .flat_map(|axis| {
            [
                Plane {
                    normal: axis,
                    dist: axis.dot(bounds.max),
                },
                Plane {
                    normal: -axis,
                    dist: -axis.dot(bounds.min),
                },
            ]
        })
        .collect()
}

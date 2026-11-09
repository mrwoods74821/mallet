//! Wireframe geometry prepared once per map change and shared by all four viewports,
//! plus hit testing for selection.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use eframe::egui::Color32;
use glam::{DMat3, DMat4, DVec2, DVec3};
use mallet_content::{EntityClass, GameData, Model};
use mallet_vmf::geometry::{faces_bounds, solid_faces, Bounds, Plane};
use mallet_vmf::light::MapLighting;
use mallet_vmf::{parse_vec3, Entity, Side, Solid, TexAxis, Vmf};

use crate::view2d::Axis2D;

/// Colour used when a brush or entity has no editor colour.
const DEFAULT_COLOR: Color32 = Color32::from_rgb(220, 220, 220);

/// Half-size of the box drawn for point entities until FGD sizes are loaded.
const POINT_ENTITY_HALF_SIZE: f64 = 8.0;

/// The model the 3D view shows for an entity: its own `model` key when that is a studio
/// model, or else the editor model its FGD class names with `studio("...")`, such as
/// the player figure of `info_player_start`.
pub fn entity_model<'a>(entity: &'a Entity, class: Option<&'a EntityClass>) -> Option<&'a str> {
    entity
        .get("model")
        .filter(|path| path.to_ascii_lowercase().ends_with(".mdl"))
        .or_else(|| class?.studio.as_deref())
}

/// The editor sprite the 3D view shows for a point entity, as a material path without
/// its extension: the icon its FGD class names with `iconsprite("...")`, such as the
/// bulb of `light` or the speaker of `ambient_generic`.
pub fn entity_sprite(class: Option<&EntityClass>) -> Option<String> {
    let icon = class?.icon.as_deref()?.trim().replace('\\', "/");
    let icon = icon.strip_prefix("materials/").unwrap_or(&icon);
    let icon = icon
        .strip_suffix(".vmt")
        .or_else(|| icon.strip_suffix(".spr"))
        .unwrap_or(icon);
    (!icon.is_empty()).then(|| icon.to_ascii_lowercase())
}

/// The box of a point entity without an FGD size, relative to its origin, as the
/// Half-Life 2 FGD gives it:
/// the standing player hull for spawn points and NPCs, a small cube otherwise.
fn point_entity_box(classname: &str) -> (DVec3, DVec3) {
    let classname = classname.to_ascii_lowercase();
    if classname.starts_with("info_player_") || classname.starts_with("npc_") {
        (DVec3::new(-16.0, -16.0, 0.0), DVec3::new(16.0, 16.0, 72.0))
    } else {
        let half = DVec3::splat(POINT_ENTITY_HALF_SIZE);
        (-half, half)
    }
}

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

/// Half the side length, in pixels, of a selection handle.
pub const HANDLE_HALF_SIZE: f32 = 3.5;

/// An affine transform previewed on the selection while it is dragged: a move, a resize
/// from the handles, or a rotation, skew or gizmo drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remap {
    pub matrix: DMat4,
}

impl Remap {
    /// Moves by `offset`.
    pub fn translate(offset: DVec3) -> Self {
        Self {
            matrix: DMat4::from_translation(offset),
        }
    }

    /// Stretches and shifts each axis so the box `from` lands on the box `to`. Along an
    /// axis where `from` is flat, points are only shifted.
    pub fn boxes(from: &Bounds, to: &Bounds) -> Self {
        let (from_size, to_size) = (from.size(), to.size());
        let scale = DVec3::select(
            from_size.cmpgt(DVec3::ZERO),
            to_size / from_size.max(DVec3::splat(f64::MIN_POSITIVE)),
            DVec3::ONE,
        );
        Self {
            matrix: DMat4::from_translation(to.min - from.min * scale) * DMat4::from_scale(scale),
        }
    }

    pub fn apply(&self, point: DVec3) -> DVec3 {
        self.matrix.transform_point3(point)
    }

    /// The same mapping as a matrix, to apply on top of a model's transform.
    pub fn matrix(&self) -> DMat4 {
        self.matrix
    }

    /// The box around a transformed box's eight corners.
    pub fn map_bounds(&self, bounds: &Bounds) -> Bounds {
        let mut mapped = Bounds::EMPTY;
        for corner in 0..8 {
            let pick = |bit: usize, axis: usize| {
                if corner & bit == 0 {
                    bounds.min[axis]
                } else {
                    bounds.max[axis]
                }
            };
            mapped.add_point(self.apply(DVec3::new(pick(1, 0), pick(2, 1), pick(4, 2))));
        }
        mapped
    }

    /// How far the transform moves the origin, which is the whole of a plain move.
    pub fn offset(&self) -> DVec3 {
        self.matrix.w_axis.truncate()
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
    /// The entity tool's position for a new entity, waiting for Enter.
    pub entity: Option<DVec3>,
    /// Faces selected with the texture tool are drawn without their red tint.
    pub hide_face_mask: bool,
    /// What the selection's handles do in the 2D views.
    pub handle_mode: crate::transform::HandleMode,
    /// Where rotations turn around, when placed away from the selection's centre.
    pub pivot: Option<DVec3>,
}

/// Half-size of the marker the entity tool draws where a new entity will go.
const ENTITY_MARKER_HALF_SIZE: f64 = 8.0;

/// The edges of the entity tool's marker: a small box around the point with a cross
/// through it, so it is visible from any side.
pub fn entity_marker_edges(point: DVec3) -> Vec<[DVec3; 2]> {
    let half = DVec3::splat(ENTITY_MARKER_HALF_SIZE);
    let mut edges = box_edges(&Bounds {
        min: point - half,
        max: point + half,
    });
    for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
        let arm = axis * ENTITY_MARKER_HALF_SIZE * 2.0;
        edges.push([point - arm, point + arm]);
    }
    edges
}

impl WireMesh {
    /// Colour to draw this mesh with and where to move it, given the current overlay.
    pub fn appearance(&self, overlay: &Overlay) -> (Color32, Option<Remap>) {
        if self.selected {
            (crate::style::current().selection, overlay.transform)
        } else {
            (self.color, None)
        }
    }

    /// The mesh bounds after an optional remap.
    pub fn mapped_bounds(&self, remap: Option<&Remap>) -> Bounds {
        match remap {
            Some(remap) => remap.map_bounds(&self.bounds),
            None => self.bounds,
        }
    }
}

/// One polygon of a brush or point entity box, for the textured 3D view.
#[derive(Clone)]
pub struct SceneFace {
    /// The face's material, or `None` for point entity boxes drawn in a flat colour.
    pub material: Option<String>,
    /// Convex polygon in winding order.
    pub vertices: Vec<DVec3>,
    /// Outward unit normal.
    pub normal: DVec3,
    pub uaxis: TexAxis,
    pub vaxis: TexAxis,
    /// The brush side this face shows, by id, so the texture tool can pick and edit it.
    pub side: Option<u32>,
    /// The brush the side belongs to.
    pub solid: Option<u32>,
    /// Whether the texture tool has this face selected.
    pub selected: bool,
    /// Units per luxel of the face's lightmap (the side's `lightmapscale`).
    pub lightmap_scale: f64,
}

/// A face hit by a ray, for the texture tool.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceHit {
    pub side: u32,
    pub solid: u32,
    /// Where the ray met the face.
    pub point: DVec3,
}

/// Loaded prop models by [`model_key`]; `None` when a model could not be loaded.
pub type ModelCache = HashMap<String, Option<Arc<Model>>>;

/// The cache key for a model path: lower case with forward slashes.
pub fn model_key(path: &str) -> String {
    path.trim().replace('\\', "/").to_ascii_lowercase()
}

/// A prop entity's model placed in the world.
#[derive(Clone, Debug)]
pub struct ModelInstance {
    /// Key into the [`ModelCache`].
    pub key: String,
    /// Model space to world: origin, then angles, then `modelscale`.
    pub transform: DMat4,
    /// Whether the model blocks light in the lighting preview: a prop's own model does
    /// unless `disableshadows` is set, while the editor models FGD classes show, such as
    /// a projected texture's lamp, are not in the game and do not.
    pub casts_shadows: bool,
    /// An editor model from the FGD class, such as an AI node's cube, rather than the
    /// entity's own model.
    pub helper: bool,
}

/// The geometry of one brush or point entity.
#[derive(Clone)]
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
    /// The prop model drawn for this entity in the 3D view, instead of `faces`.
    pub model: Option<ModelInstance>,
    /// The editor sprite drawn for this point entity in the 3D view, as a material path
    /// such as `editor/light`. Once its texture loads, the sprite replaces the entity's
    /// flat box, the way Hammer shows lights, sounds and cube maps.
    pub sprite: Option<String>,
    /// The number of the first of `faces` among all the scene's faces, counting mesh by
    /// mesh, so a renderer can keep data for each face; `None` for meshes made outside
    /// [`Scene::build`].
    pub first_face: Option<u32>,
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
    /// Recent changes that only touched materials and alignment, oldest first, so a
    /// renderer that has drawn generation `from` of one can follow the chain to the
    /// current generation and update just those meshes.
    pub retextures: Vec<Retexture>,
    /// The map's lights, sky, fog and glows, for the lighting preview.
    pub lighting: MapLighting,
}

/// One step of [`Scene::retextures`]: these meshes changed from `from` to `to`.
#[derive(Debug, Clone)]
pub struct Retexture {
    pub from: u64,
    pub to: u64,
    pub meshes: Vec<usize>,
}

/// How many retexture steps a scene remembers; a renderer further behind rebuilds all.
const MAX_RETEXTURES: usize = 64;

impl Scene {
    /// Builds wireframes for every visible brush and point entity in the map.
    ///
    /// World brushes use their own editor colour; brushes of brush entities use the
    /// entity's colour, matching Hammer's 2D views. Entities with a loaded `model` get the
    /// model, outlined by its rotated bounding box; others get the box their FGD class
    /// gives, or a small box. Point entities without their own colour use the class's.
    pub fn build(vmf: &Vmf, models: &ModelCache, game_data: Option<&GameData>) -> Self {
        let mut scene = Self {
            geometry_generation: next_generation(),
            selection_generation: next_generation(),
            lighting: MapLighting::read(vmf),
            ..Self::default()
        };
        for entity in vmf.all_entities() {
            let is_world = entity.classname().eq_ignore_ascii_case("worldspawn");
            if !is_world {
                scene.entity_count += 1;
            }
            let class = game_data.and_then(|data| data.class(entity.classname()));
            let entity_color = entity.editor_color().or_else(|| class.and_then(|class| class.color));
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
            let loaded = entity_model(entity, class)
                .map(model_key)
                .and_then(|key| Some((models.get(&key)?.clone()?, key)));
            if let (Some(origin), Some((model, key))) = (entity.origin(), loaded) {
                let transform = if class.is_some_and(|class| class.light_prop) {
                    light_prop_transform(origin, entity)
                } else {
                    entity_transform(origin, entity)
                };
                let local = Bounds {
                    min: DVec3::from_array(model.min.map(f64::from)),
                    max: DVec3::from_array(model.max.map(f64::from)),
                };
                let edges: Vec<[DVec3; 2]> = box_edges(&local)
                    .into_iter()
                    .map(|[a, b]| [transform.transform_point3(a), transform.transform_point3(b)])
                    .collect();
                let mut bounds = Bounds::EMPTY;
                for point in edges.iter().flatten() {
                    bounds.add_point(*point);
                }
                let own_model = entity
                    .get("model")
                    .is_some_and(|path| path.trim().to_ascii_lowercase().ends_with(".mdl"));
                let shadows_off = entity.get("disableshadows").is_some_and(|value| value.trim() == "1");
                let mesh = WireMesh {
                    color: to_color(entity_color),
                    edges,
                    faces: Vec::new(),
                    planes: box_planes(&bounds),
                    bounds,
                    owner: entity_owner,
                    selected: false,
                    model: Some(ModelInstance {
                        key,
                        transform,
                        casts_shadows: own_model && !shadows_off,
                        helper: !own_model,
                    }),
                    sprite: None,
                    first_face: None,
                };
                scene.bounds.add_bounds(&mesh.bounds);
                scene.meshes.push(mesh);
            } else if let Some(origin) = entity.origin() {
                let (min, max) = match class.and_then(|class| class.size) {
                    Some((min, max)) => (DVec3::from_array(min), DVec3::from_array(max)),
                    None => point_entity_box(entity.classname()),
                };
                let bounds = Bounds {
                    min: origin + min,
                    max: origin + max,
                };
                let mesh = WireMesh {
                    color: to_color(entity_color),
                    edges: box_edges(&bounds),
                    faces: box_faces(&bounds),
                    bounds,
                    planes: box_planes(&bounds),
                    owner: entity_owner,
                    selected: false,
                    model: None,
                    sprite: entity_sprite(class),
                    first_face: None,
                };
                scene.bounds.add_bounds(&mesh.bounds);
                scene.meshes.push(mesh);
            }
        }
        let mut faces = 0u32;
        for mesh in &mut scene.meshes {
            mesh.first_face = Some(faces);
            faces = faces.saturating_add(mesh.faces.len() as u32);
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
        let on_centre =
            |mesh: &&WireMesh| (axis.project(mesh.bounds.center()) - point).abs().max_element() <= tolerance;
        let on_outline = |mesh: &&WireMesh| {
            let min = axis.project(mesh.bounds.min) - DVec2::splat(tolerance);
            let max = axis.project(mesh.bounds.max) + DVec2::splat(tolerance);
            point.cmpge(min).all()
                && point.cmple(max).all()
                && mesh
                    .edges
                    .iter()
                    .any(|[a, b]| segment_distance(point, axis.project(*a), axis.project(*b)) <= tolerance)
        };
        smallest(&mut self.meshes.iter().filter(on_centre))
            .or_else(|| smallest(&mut self.meshes.iter().filter(on_outline)))
    }

    /// Picks the nearest mesh hit by a ray, the way Hammer does: a prop counts where the
    /// ray meets its model's triangles, so clicking through a gap in a fence picks what is
    /// behind it. Only when the ray hits nothing at all is a prop picked by its bounding
    /// box, so thin or hollow models can still be clicked.
    pub fn pick_ray(&self, origin: DVec3, direction: DVec3, models: &ModelCache) -> Option<Owner> {
        let nearest = |hits: &mut dyn Iterator<Item = (Owner, f64)>| {
            hits.min_by(|a, b| a.1.total_cmp(&b.1)).map(|(owner, _)| owner)
        };
        nearest(
            &mut self
                .meshes
                .iter()
                .filter_map(|mesh| Some((mesh.owner?, mesh_hit(mesh, origin, direction, models)?))),
        )
        .or_else(|| {
            nearest(
                &mut self
                    .meshes
                    .iter()
                    .filter(|mesh| mesh.model.is_some())
                    .filter_map(|mesh| Some((mesh.owner?, ray_box(origin, direction, &mesh.bounds)?))),
            )
        })
    }

    /// The brush face a ray hits first, for the texture tool. Point entities and props
    /// block the ray but are not faces; a prop blocks it only where its model is.
    pub fn pick_face(&self, origin: DVec3, direction: DVec3, models: &ModelCache) -> Option<FaceHit> {
        let (mesh, distance, plane) = self
            .meshes
            .iter()
            .filter(|mesh| mesh.owner.is_some())
            .filter_map(|mesh| {
                if mesh.model.is_some() {
                    return Some((mesh, mesh_hit(mesh, origin, direction, models)?, None));
                }
                let (distance, plane) = ray_convex_face(origin, direction, &mesh.planes)?;
                Some((mesh, distance, plane))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))?;
        let face = mesh.faces.get(plane?)?;
        Some(FaceHit {
            side: face.side?,
            solid: face.solid?,
            point: origin + direction * distance,
        })
    }

    /// Rebuilds the meshes of brushes whose materials or texture alignment changed, keeping
    /// everything else, and notes them in [`Scene::retextures`]. Returns `false` when a
    /// brush has no mesh, so the caller should rebuild the whole scene.
    pub fn retexture(&mut self, solids: &[Solid]) -> bool {
        let index: HashMap<u32, usize> = self
            .meshes
            .iter()
            .enumerate()
            .filter_map(|(at, mesh)| Some((mesh.faces.first()?.solid?, at)))
            .collect();
        let mut changed = Vec::with_capacity(solids.len());
        for solid in solids {
            let Some(&at) = solid.id().and_then(|id| index.get(&id)) else {
                return false;
            };
            let old = &self.meshes[at];
            let Some(mut mesh) = brush_mesh(solid, old.color, old.owner) else {
                return false;
            };
            mesh.selected = old.selected;
            mesh.first_face = old.first_face.filter(|_| mesh.faces.len() == old.faces.len());
            self.meshes[at] = mesh;
            changed.push(at);
        }
        let from = self.geometry_generation;
        self.geometry_generation = next_generation();
        self.retextures.push(Retexture {
            from,
            to: self.geometry_generation,
            meshes: changed,
        });
        if self.retextures.len() > MAX_RETEXTURES {
            self.retextures.remove(0);
        }
        true
    }

    /// Marks the faces whose side ids are in `sides` as selected, for drawing.
    pub fn mark_faces(&mut self, sides: &BTreeSet<u32>) {
        for face in self.meshes.iter_mut().flat_map(|mesh| mesh.faces.iter_mut()) {
            face.selected = face.side.is_some_and(|side| sides.contains(&side));
        }
        self.selection_generation = next_generation();
    }

    /// Where a ray first hits a brush face, ignoring point entities and props. Used to
    /// place new entities on the surface under the cursor.
    pub fn surface_hit(&self, origin: DVec3, direction: DVec3) -> Option<DVec3> {
        self.meshes
            .iter()
            .filter(|mesh| mesh.model.is_none() && mesh.faces.iter().any(|face| face.material.is_some()))
            .filter_map(|mesh| ray_convex(origin, direction, &mesh.planes))
            .min_by(f64::total_cmp)
            .map(|distance| origin + direction * distance)
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
    let t = if length > 0.0 {
        ((point - a).dot(along) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.distance(a + along * t)
}

/// Distance along a ray to where it meets a mesh: the triangles of a prop's model, or
/// the planes of a brush or entity box. A prop whose model is not loaded counts as its box.
fn mesh_hit(mesh: &WireMesh, origin: DVec3, direction: DVec3, models: &ModelCache) -> Option<f64> {
    let loaded = mesh
        .model
        .as_ref()
        .and_then(|instance| Some((instance, models.get(&instance.key)?.as_deref()?)));
    match loaded {
        Some((instance, model)) => ray_model(origin, direction, &mesh.bounds, instance, model),
        None => ray_convex(origin, direction, &mesh.planes),
    }
}

/// Distance along a ray to the nearest triangle of a placed model, from either side.
fn ray_model(origin: DVec3, direction: DVec3, bounds: &Bounds, instance: &ModelInstance, model: &Model) -> Option<f64> {
    ray_box(origin, direction, bounds)?;
    if instance.transform.determinant().abs() < 1e-12 {
        return None;
    }
    // An affine transform keeps distances along the ray in proportion, so the distance
    // found in model space is the distance in the world.
    let inverse = instance.transform.inverse();
    let local_origin = inverse.transform_point3(origin);
    let local_direction = inverse.transform_vector3(direction);
    let corner = |index: u32| {
        model
            .vertices
            .get(index as usize)
            .map(|vertex| DVec3::from_array(vertex.position.map(f64::from)))
    };
    model
        .meshes
        .iter()
        .flat_map(|mesh| mesh.indices.as_chunks::<3>().0)
        .filter_map(|&[a, b, c]| {
            let [a, b, c] = [corner(a)?, corner(b)?, corner(c)?];
            ray_triangle(local_origin, local_direction, a, b, c)
        })
        .min_by(f64::total_cmp)
}

/// Möller-Trumbore: distance along a ray to a triangle, hit from either side.
fn ray_triangle(origin: DVec3, direction: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Option<f64> {
    let edge1 = b - a;
    let edge2 = c - a;
    let p = direction.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() < 1e-12 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let s = origin - a;
    let u = s.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(edge1);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = edge2.dot(q) * inverse;
    (t > 0.0).then_some(t)
}

/// Distance along a ray to an axis-aligned box, or 0 when it starts inside.
fn ray_box(origin: DVec3, direction: DVec3, bounds: &Bounds) -> Option<f64> {
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;
    for axis in 0..3 {
        let (start, step) = (origin[axis], direction[axis]);
        let (low, high) = (bounds.min[axis], bounds.max[axis]);
        if step.abs() < 1e-12 {
            if start < low || start > high {
                return None;
            }
            continue;
        }
        let (t1, t2) = ((low - start) / step, (high - start) / step);
        near = near.max(t1.min(t2));
        far = far.min(t1.max(t2));
    }
    (near <= far && far >= 0.0).then_some(near.max(0.0))
}

/// Distance along the ray to where it enters a convex volume, or `None` on a miss.
/// A ray starting inside the volume misses it, so clicking from inside a brush picks
/// what is beyond it, as faces seen from inside are not drawn either.
fn ray_convex(origin: DVec3, direction: DVec3, planes: &[Plane]) -> Option<f64> {
    ray_convex_face(origin, direction, planes).map(|(distance, _)| distance)
}

/// Like [`ray_convex`], also giving the index of the plane the ray enters through.
fn ray_convex_face(origin: DVec3, direction: DVec3, planes: &[Plane]) -> Option<(f64, Option<usize>)> {
    if planes.is_empty() || planes.iter().all(|plane| plane.distance(origin) <= 0.0) {
        return None;
    }
    let mut enter: f64 = 0.0;
    let mut entered = None;
    let mut exit = f64::INFINITY;
    for (index, plane) in planes.iter().enumerate() {
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
            if t > enter || entered.is_none() {
                enter = enter.max(t);
                entered = Some(index);
            }
        } else {
            exit = exit.min(t);
        }
        if enter > exit {
            return None;
        }
    }
    Some((enter, entered))
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
    // Faces and planes stay in the same order, so the plane a ray enters through names
    // the face it hit.
    let (scene_faces, planes): (Vec<SceneFace>, Vec<Plane>) = faces
        .iter()
        .filter_map(|face| {
            let side = sides.get(face.side)?;
            let scene_face = SceneFace {
                material: Some(side.material.clone()),
                vertices: face.vertices.clone(),
                normal: face.plane.normal,
                uaxis: side.uaxis,
                vaxis: side.vaxis,
                side: Some(side.id),
                solid: solid.id(),
                selected: false,
                lightmap_scale: f64::from(side.lightmap_scale.unwrap_or(16).max(1)),
            };
            Some((scene_face, face.plane))
        })
        .unzip();
    Some(WireMesh {
        color,
        edges,
        faces: scene_faces,
        bounds: faces_bounds(&faces),
        planes,
        owner,
        selected: false,
        model: None,
        sprite: None,
        first_face: None,
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
            let (u, v) = if normal.z.abs() > 0.5 {
                (DVec3::X, DVec3::Y)
            } else {
                (DVec3::Z, normal.cross(DVec3::Z))
            };
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
                side: None,
                solid: None,
                selected: false,
                lightmap_scale: 16.0,
            }
        })
        .collect()
}

/// Model space to world for a prop: `origin`, then `angles` (pitch, yaw, roll in degrees,
/// applied roll first, then pitch, then yaw, like the engine), then `modelscale`.
pub fn entity_transform(origin: DVec3, entity: &Entity) -> DMat4 {
    let [pitch, yaw, roll] = entity
        .get("angles")
        .and_then(parse_vec3)
        .map_or([0.0; 3], |angles| angles.to_array().map(f64::to_radians));
    let scale = entity
        .get("modelscale")
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|scale| *scale > 0.0)
        .unwrap_or(1.0);
    let rotation = DMat3::from_rotation_z(yaw) * DMat3::from_rotation_y(pitch) * DMat3::from_rotation_x(roll);
    DMat4::from_translation(origin) * DMat4::from_mat3(rotation) * DMat4::from_scale(DVec3::splat(scale))
}

/// Model space to world for a `lightprop()` model, such as light_spot's lamp: turned so
/// its front points the way the light shines, as the compiler reads `angles`, `angle`
/// and `pitch`, keeping the roll from `angles`.
fn light_prop_transform(origin: DVec3, entity: &Entity) -> DMat4 {
    let direction = mallet_vmf::light::vrad_direction(entity);
    let roll = entity
        .get("angles")
        .and_then(parse_vec3)
        .map_or(0.0, |angles| angles.z.to_radians());
    let yaw = direction.y.atan2(direction.x);
    let pitch = -direction.z.clamp(-1.0, 1.0).asin();
    let rotation = DMat3::from_rotation_z(yaw) * DMat3::from_rotation_y(pitch) * DMat3::from_rotation_x(roll);
    DMat4::from_translation(origin) * DMat4::from_mat3(rotation)
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

pub fn next_generation() -> u64 {
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

#[cfg(test)]
mod tests {
    use mallet_content::{ModelMesh, ModelVertex};

    use super::*;

    fn box_mesh(owner: Owner, min: DVec3, max: DVec3) -> WireMesh {
        let bounds = Bounds { min, max };
        WireMesh {
            color: DEFAULT_COLOR,
            edges: box_edges(&bounds),
            faces: Vec::new(),
            planes: box_planes(&bounds),
            bounds,
            owner: Some(owner),
            selected: false,
            model: None,
            sprite: None,
            first_face: None,
        }
    }

    /// A prop whose model is one upright triangle in the plane x = 0, filling only the
    /// lower corner of its 64-unit bounding box.
    fn fence() -> (WireMesh, ModelCache) {
        let vertex = |position: [f32; 3]| ModelVertex {
            position,
            normal: [1.0, 0.0, 0.0],
            uv: [0.0, 0.0],
        };
        let model = Model {
            vertices: vec![
                vertex([0.0, -32.0, 0.0]),
                vertex([0.0, 0.0, 0.0]),
                vertex([0.0, -32.0, 32.0]),
            ],
            meshes: vec![ModelMesh {
                material: "fence".to_owned(),
                indices: vec![0, 1, 2],
            }],
            min: [-32.0, -32.0, 0.0],
            max: [32.0, 32.0, 64.0],
        };
        let transform = DMat4::from_translation(DVec3::new(100.0, 0.0, 0.0));
        let mut prop = box_mesh(
            Owner::Entity(7),
            DVec3::new(68.0, -32.0, 0.0),
            DVec3::new(132.0, 32.0, 64.0),
        );
        prop.model = Some(ModelInstance {
            key: "fence.mdl".to_owned(),
            transform,
            casts_shadows: true,
            helper: false,
        });
        let models = ModelCache::from([("fence.mdl".to_owned(), Some(Arc::new(model)))]);
        (prop, models)
    }

    #[test]
    fn props_are_picked_by_their_triangles_before_their_box() {
        let (prop, models) = fence();
        let wall = box_mesh(
            Owner::Solid(1),
            DVec3::new(200.0, -64.0, 0.0),
            DVec3::new(216.0, 64.0, 64.0),
        );
        let scene = Scene {
            meshes: vec![prop.clone(), wall],
            ..Scene::default()
        };
        let along_x = DVec3::X;
        // Through the model's triangle: the prop, even though the wall is behind it.
        let on_model = scene.pick_ray(DVec3::new(0.0, -20.0, 8.0), along_x, &models);
        assert_eq!(on_model, Some(Owner::Entity(7)));
        // Through the empty part of its box: the wall behind it.
        let through_gap = scene.pick_ray(DVec3::new(0.0, 20.0, 40.0), along_x, &models);
        assert_eq!(through_gap, Some(Owner::Solid(1)));
        // Nothing behind it: the box still picks the prop.
        let alone = Scene {
            meshes: vec![prop],
            ..Scene::default()
        };
        assert_eq!(
            alone.pick_ray(DVec3::new(0.0, 20.0, 40.0), along_x, &models),
            Some(Owner::Entity(7))
        );
        assert_eq!(alone.pick_ray(DVec3::new(0.0, 200.0, 40.0), along_x, &models), None);
    }
}

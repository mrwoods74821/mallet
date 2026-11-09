//! Brushes as vertices, edges and faces, for Hammer's vertex tool.
//!
//! A [`BrushMesh`] is built from a brush's planes once, edited by moving its vertices or
//! splitting its faces, and turned back into planes with [`BrushMesh::build`]. Faces that
//! a move bends out of their plane are cut into triangles, vertices dragged onto each
//! other become one, and shapes that are no longer convex are refused, so every brush
//! the tool writes is one VBSP can compile.

use std::fmt;

use glam::DVec3;
use mallet_kv::Item;

use crate::geometry::{solid_faces, Plane};
use crate::{IdAllocator, Side, Solid, SolidItem};

/// Vertices closer than this are the same vertex.
pub const WELD_DISTANCE: f64 = 0.01;

/// How far a vertex may sit off its face's plane before the face counts as bent.
const PLANAR_DISTANCE: f64 = 0.01;

/// How far a vertex may sit outside another face's plane in a convex brush.
const CONVEX_DISTANCE: f64 = 0.02;

/// How far the rebuilt brush's corners may be from the mesh's vertices.
const MATCH_DISTANCE: f64 = 0.05;

/// Normals closer than this in cosine belong to the same plane.
const SAME_PLANE_COSINE: f64 = 1.0 - 1e-6;

/// One face of a mesh: the brush side its material and texture come from, and its
/// vertices in winding order.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshFace {
    /// Index among the original brush's sides.
    pub side: usize,
    /// Indices into [`BrushMesh::points`].
    pub vertices: Vec<usize>,
}

/// A convex brush as shared vertices and the faces between them.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushMesh {
    pub points: Vec<DVec3>,
    pub faces: Vec<MeshFace>,
}

/// Why the vertex tool cannot edit a brush or make a shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexError {
    /// The brush has a displacement, whose surface would no longer fit its face.
    Displacement,
    /// The brush's planes do not enclose a volume.
    InvalidBrush,
    /// The shape is not convex, or too flat to be a brush.
    NotConvex,
    /// The two vertices do not share a face.
    NotOnOneFace,
    /// The two vertices are already joined by an edge.
    AlreadyJoined,
}

impl fmt::Display for VertexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Displacement => "brushes with displacements cannot be vertex edited",
            Self::InvalidBrush => "the brush is invalid",
            Self::NotConvex => "the brush would not be convex",
            Self::NotOnOneFace => "the two vertices are not on the same face",
            Self::AlreadyJoined => "the two vertices are already joined by an edge",
        })
    }
}

impl std::error::Error for VertexError {}

impl BrushMesh {
    /// The vertices and faces of a brush.
    pub fn from_solid(solid: &Solid) -> Result<Self, VertexError> {
        if solid.sides().any(|side| side.dispinfo().is_some()) {
            return Err(VertexError::Displacement);
        }
        let faces = solid_faces(solid);
        if faces.len() < 4 {
            return Err(VertexError::InvalidBrush);
        }
        let mut points: Vec<DVec3> = Vec::new();
        let mut mesh_faces = Vec::with_capacity(faces.len());
        for face in faces {
            let vertices = face
                .vertices
                .iter()
                .map(|&vertex| weld_index(&mut points, vertex))
                .collect();
            if let Some(vertices) = clean_loop(vertices) {
                mesh_faces.push(MeshFace {
                    side: face.side,
                    vertices,
                });
            }
        }
        Ok(Self {
            points,
            faces: mesh_faces,
        })
    }

    /// Every edge once, as pairs of vertex indices with the smaller first.
    pub fn edges(&self) -> Vec<[usize; 2]> {
        let mut edges = Vec::new();
        for face in &self.faces {
            for (index, &a) in face.vertices.iter().enumerate() {
                let b = face.vertices[(index + 1) % face.vertices.len()];
                let edge = [a.min(b), a.max(b)];
                if !edges.contains(&edge) {
                    edges.push(edge);
                }
            }
        }
        edges
    }

    /// A copy with some vertices placed elsewhere, given as `(vertex, position)` pairs.
    pub fn moved(&self, moves: &[(usize, DVec3)]) -> Self {
        let mut mesh = self.clone();
        for &(vertex, position) in moves {
            if let Some(point) = mesh.points.get_mut(vertex) {
                *point = position;
            }
        }
        mesh
    }

    /// Whether any of `vertices` lies on another vertex, so building would merge them.
    pub fn overlaps(&self, vertices: &[usize]) -> bool {
        vertices.iter().any(|&vertex| {
            self.points
                .iter()
                .enumerate()
                .any(|(other, point)| other != vertex && point.distance(self.points[vertex]) < WELD_DISTANCE)
        })
    }

    /// Cuts the face that holds both vertices in two along the line between them, as
    /// Hammer's Split Face does. Both halves keep the face's material; they stay one plane
    /// until a vertex is moved off it.
    pub fn split_face(&self, a: usize, b: usize) -> Result<Self, VertexError> {
        let mut shared = false;
        for (index, face) in self.faces.iter().enumerate() {
            let (Some(at_a), Some(at_b)) = (
                face.vertices.iter().position(|&vertex| vertex == a),
                face.vertices.iter().position(|&vertex| vertex == b),
            ) else {
                continue;
            };
            shared = true;
            let count = face.vertices.len();
            let (first, second) = (at_a.min(at_b), at_a.max(at_b));
            if second - first == 1 || (first == 0 && second == count - 1) {
                continue;
            }
            let half = |from: usize, to: usize| {
                let mut vertices = Vec::new();
                let mut at = from;
                loop {
                    vertices.push(face.vertices[at]);
                    if at == to {
                        break;
                    }
                    at = (at + 1) % count;
                }
                MeshFace {
                    side: face.side,
                    vertices,
                }
            };
            let mut mesh = self.clone();
            mesh.faces[index] = half(first, second);
            mesh.faces.insert(index + 1, half(second, first));
            return Ok(mesh);
        }
        Err(if shared {
            VertexError::AlreadyJoined
        } else {
            VertexError::NotOnOneFace
        })
    }

    /// Turns the mesh back into a brush with `original`'s id, keys and materials.
    ///
    /// Vertices on top of each other are merged, bent faces are cut into triangles and
    /// faces left in one plane become one side again. Sides keep their texture axes, so
    /// textures stay where they are in the world. Returns the brush and the mesh as it
    /// was built, which keeps split faces apart for further editing and numbers its
    /// faces by the new brush's sides.
    pub fn build(&self, original: &Solid, ids: &mut IdAllocator) -> Result<(Solid, BrushMesh), VertexError> {
        let welded = self.welded();
        if welded.points.len() < 4 {
            return Err(VertexError::NotConvex);
        }
        let center = welded.points.iter().copied().sum::<DVec3>() / welded.points.len() as f64;
        let mut faces = Vec::new();
        for face in &welded.faces {
            if welded.is_planar(&face.vertices) {
                faces.push(face.clone());
            } else {
                faces.extend(welded.triangulate(face, center));
            }
        }
        let mesh = BrushMesh {
            points: welded.points,
            faces,
        };
        let planes: Vec<Plane> = mesh
            .faces
            .iter()
            .map(|face| mesh.outward_plane(&face.vertices, center))
            .collect::<Option<_>>()
            .ok_or(VertexError::NotConvex)?;
        let convex = planes.iter().all(|plane| {
            mesh.points
                .iter()
                .all(|&point| plane.distance(point) <= CONVEX_DISTANCE)
        });
        if !convex {
            return Err(VertexError::NotConvex);
        }
        let groups = plane_groups(&planes);
        if groups.len() < 4 {
            return Err(VertexError::NotConvex);
        }
        let originals: Vec<&Side> = original.sides().collect();
        let mut used = vec![false; originals.len()];
        let mut sides = Vec::with_capacity(groups.len());
        for group in &groups {
            let first = &mesh.faces[group[0]];
            let template = originals.get(first.side).ok_or(VertexError::InvalidBrush)?;
            let mut side = (*template).clone();
            if std::mem::replace(&mut used[first.side], true) {
                side.id = ids.side();
            }
            let corners: Vec<DVec3> = group
                .iter()
                .flat_map(|&face| &mesh.faces[face].vertices)
                .map(|&vertex| mesh.points[vertex])
                .collect();
            side.plane = plane_points(&corners, planes[group[0]].normal).ok_or(VertexError::NotConvex)?;
            drop_precise_vertices(&mut side);
            sides.push(side);
        }
        let solid = with_sides(original, sides);
        if !mesh.matches(&solid) {
            return Err(VertexError::NotConvex);
        }
        let mut mesh = mesh;
        for (index, group) in groups.iter().enumerate() {
            for &face in group {
                mesh.faces[face].side = index;
            }
        }
        Ok((solid, mesh))
    }

    /// The mesh with vertices on top of each other merged and faces that collapse dropped.
    fn welded(&self) -> Self {
        let mut points = Vec::new();
        let remap: Vec<usize> = self
            .points
            .iter()
            .map(|&point| weld_index(&mut points, point))
            .collect();
        let faces = self
            .faces
            .iter()
            .filter_map(|face| {
                let vertices = clean_loop(face.vertices.iter().map(|&vertex| remap[vertex]).collect())?;
                Some(MeshFace {
                    side: face.side,
                    vertices,
                })
            })
            .collect();
        Self { points, faces }
    }

    fn is_planar(&self, vertices: &[usize]) -> bool {
        let Some((normal, middle)) = self.loop_normal(vertices) else {
            return true;
        };
        let normal = normal.normalize();
        vertices
            .iter()
            .all(|&vertex| normal.dot(self.points[vertex] - middle).abs() <= PLANAR_DISTANCE)
    }

    /// The loop's area-weighted normal by Newell's method and its average point, or
    /// `None` when it has no area.
    fn loop_normal(&self, vertices: &[usize]) -> Option<(DVec3, DVec3)> {
        let middle = vertices.iter().map(|&vertex| self.points[vertex]).sum::<DVec3>() / vertices.len() as f64;
        let mut normal = DVec3::ZERO;
        for (index, &vertex) in vertices.iter().enumerate() {
            let next = vertices[(index + 1) % vertices.len()];
            normal += (self.points[vertex] - middle).cross(self.points[next] - middle);
        }
        (normal.length() > 1e-9).then_some((normal, middle))
    }

    /// The face's plane, facing away from `center`.
    fn outward_plane(&self, vertices: &[usize], center: DVec3) -> Option<Plane> {
        let (normal, middle) = self.loop_normal(vertices)?;
        let mut normal = normal.normalize();
        if normal.dot(middle - center) < 0.0 {
            normal = -normal;
        }
        Some(Plane {
            normal,
            dist: normal.dot(middle),
        })
    }

    /// Cuts a bent face into a fan of triangles. Of the fans from each corner, the first
    /// that keeps the brush convex is used.
    fn triangulate(&self, face: &MeshFace, center: DVec3) -> Vec<MeshFace> {
        let count = face.vertices.len();
        let fan = |start: usize| -> Vec<MeshFace> {
            (1..count - 1)
                .map(|step| MeshFace {
                    side: face.side,
                    vertices: vec![
                        face.vertices[start],
                        face.vertices[(start + step) % count],
                        face.vertices[(start + step + 1) % count],
                    ],
                })
                .collect()
        };
        let convex = |triangles: &[MeshFace]| {
            triangles.iter().all(|triangle| {
                self.outward_plane(&triangle.vertices, center).is_some_and(|plane| {
                    self.points
                        .iter()
                        .all(|&point| plane.distance(point) <= CONVEX_DISTANCE)
                })
            })
        };
        (0..count)
            .map(fan)
            .find(|triangles| convex(triangles))
            .unwrap_or_else(|| fan(0))
    }

    /// Whether the brush's own corners are exactly the mesh's vertices.
    fn matches(&self, solid: &Solid) -> bool {
        let faces = solid_faces(solid);
        if faces.is_empty() {
            return false;
        }
        let corners: Vec<DVec3> = faces.iter().flat_map(|face| face.vertices.iter().copied()).collect();
        let near = |a: &DVec3, list: &[DVec3]| list.iter().any(|b| a.distance(*b) <= MATCH_DISTANCE);
        corners.iter().all(|corner| near(corner, &self.points)) && self.points.iter().all(|point| near(point, &corners))
    }
}

/// The index of the point in `points` within weld distance of `point`, adding it first
/// when there is none.
fn weld_index(points: &mut Vec<DVec3>, point: DVec3) -> usize {
    match points
        .iter()
        .position(|existing| existing.distance(point) < WELD_DISTANCE)
    {
        Some(index) => index,
        None => {
            points.push(point);
            points.len() - 1
        }
    }
}

/// A face loop without repeated neighbours, or `None` when fewer than three corners remain.
fn clean_loop(mut vertices: Vec<usize>) -> Option<Vec<usize>> {
    vertices.dedup();
    while vertices.len() > 1 && vertices.first() == vertices.last() {
        vertices.pop();
    }
    (vertices.len() >= 3).then_some(vertices)
}

/// Faces grouped by the plane they lie in, each group in first-seen order.
fn plane_groups(planes: &[Plane]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (index, plane) in planes.iter().enumerate() {
        let same = groups.iter_mut().find(|group| {
            let other = planes[group[0]];
            other.normal.dot(plane.normal) >= SAME_PLANE_COSINE && (other.dist - plane.dist).abs() <= PLANAR_DISTANCE
        });
        match same {
            Some(group) => group.push(index),
            None => groups.push(vec![index]),
        }
    }
    groups
}

/// Three of the corners spanning the largest triangle, in Hammer's winding for a plane
/// facing along `normal`. Using real corners keeps grid-aligned brushes exact.
fn plane_points(corners: &[DVec3], normal: DVec3) -> Option<[DVec3; 3]> {
    let mut best = None;
    let mut best_area = 1e-6;
    for (i, &a) in corners.iter().enumerate() {
        for (j, &b) in corners.iter().enumerate().skip(i + 1) {
            for &c in corners.iter().skip(j + 1) {
                let area = (a - b).cross(c - b).length();
                if area > best_area {
                    best_area = area;
                    best = Some([a, b, c]);
                }
            }
        }
    }
    let [a, b, c] = best?;
    Some(if (a - b).cross(c - b).dot(normal) >= 0.0 {
        [a, b, c]
    } else {
        [c, b, a]
    })
}

/// Removes Hammer++'s `vertices_plus` block, which would describe the old shape.
fn drop_precise_vertices(side: &mut Side) {
    let mut index = 0;
    while index < side.extra.len() {
        let stale =
            matches!(&side.extra[index], Item::Block(block) if block.name.eq_ignore_ascii_case("vertices_plus"));
        if stale {
            side.extra.remove(index);
            if index < side.extra_slots.len() {
                side.extra_slots.remove(index);
            }
        } else {
            index += 1;
        }
    }
}

/// `original` with its sides replaced by `sides`, placed where its first side was.
fn with_sides(original: &Solid, sides: Vec<Side>) -> Solid {
    let mut items = Vec::with_capacity(original.items.len() + sides.len());
    let mut sides = Some(sides);
    for item in &original.items {
        match item {
            SolidItem::Side(_) => {
                if let Some(sides) = sides.take() {
                    items.extend(sides.into_iter().map(|side| SolidItem::Side(Box::new(side))));
                }
            }
            other => items.push(other.clone()),
        }
    }
    if let Some(sides) = sides {
        items.extend(sides.into_iter().map(|side| SolidItem::Side(Box::new(side))));
    }
    Solid { items }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::box_solid;
    use crate::Vmf;

    fn cube() -> (Solid, IdAllocator) {
        let mut ids = IdAllocator::for_map(&Vmf::new_map());
        let solid = box_solid(DVec3::ZERO, DVec3::splat(64.0), "DEV/DEV_MEASUREGENERIC01B", &mut ids).unwrap();
        (solid, ids)
    }

    fn vertex(mesh: &BrushMesh, at: DVec3) -> usize {
        mesh.points.iter().position(|point| *point == at).unwrap()
    }

    fn corners(solid: &Solid) -> Vec<DVec3> {
        let mut points: Vec<DVec3> = Vec::new();
        for face in solid_faces(solid) {
            for vertex in face.vertices {
                if !points.iter().any(|point| point.distance(vertex) < WELD_DISTANCE) {
                    points.push(vertex);
                }
            }
        }
        points
    }

    #[test]
    fn a_cube_has_eight_vertices_twelve_edges_and_six_faces() {
        let (solid, _) = cube();
        let mesh = BrushMesh::from_solid(&solid).unwrap();
        assert_eq!(mesh.points.len(), 8);
        assert_eq!(mesh.edges().len(), 12);
        assert_eq!(mesh.faces.len(), 6);
    }

    #[test]
    fn building_an_unchanged_mesh_keeps_the_shape_and_ids() {
        let (solid, mut ids) = cube();
        let mesh = BrushMesh::from_solid(&solid).unwrap();
        let (built, _) = mesh.build(&solid, &mut ids).unwrap();
        assert_eq!(built.id(), solid.id());
        let before: Vec<u32> = solid.sides().map(|side| side.id).collect();
        let after: Vec<u32> = built.sides().map(|side| side.id).collect();
        assert_eq!(before, after);
        let mut a = corners(&solid);
        let mut b = corners(&built);
        let key = |p: &DVec3| (p.x as i64, p.y as i64, p.z as i64);
        a.sort_by_key(key);
        b.sort_by_key(key);
        assert_eq!(a, b);
    }

    #[test]
    fn raising_a_corner_cuts_its_faces_into_triangles() {
        let (solid, mut ids) = cube();
        let mesh = BrushMesh::from_solid(&solid).unwrap();
        let top = vertex(&mesh, DVec3::splat(64.0));
        let (built, rebuilt) = mesh
            .moved(&[(top, DVec3::new(64.0, 64.0, 96.0))])
            .build(&solid, &mut ids)
            .unwrap();
        assert!(corners(&built).contains(&DVec3::new(64.0, 64.0, 96.0)));
        assert_eq!(corners(&built).len(), 8);
        assert!(built.sides().count() > 6);
        assert!(rebuilt.faces.iter().any(|face| face.vertices.len() == 3));
        let mut seen = std::collections::HashSet::new();
        assert!(built.sides().all(|side| seen.insert(side.id)));
    }

    #[test]
    fn faces_keep_their_materials_over_several_edits() {
        let (mut solid, mut ids) = cube();
        for (index, side) in solid.sides_mut().enumerate() {
            side.material = format!("FACE{index}");
        }
        let material_facing = |solid: &Solid, normal: DVec3| -> String {
            let faces = solid_faces(solid);
            let face = faces.iter().find(|face| face.plane.normal.dot(normal) > 0.999).unwrap();
            solid.sides().nth(face.side).unwrap().material.clone()
        };
        let bottom = material_facing(&solid, DVec3::NEG_Z);
        let mesh = BrushMesh::from_solid(&solid).unwrap();
        let top = vertex(&mesh, DVec3::splat(64.0));
        let (first, mesh) = mesh
            .moved(&[(top, DVec3::new(64.0, 64.0, 80.0))])
            .build(&solid, &mut ids)
            .unwrap();
        let top = vertex(&mesh, DVec3::new(64.0, 64.0, 80.0));
        let (second, _) = mesh
            .moved(&[(top, DVec3::new(64.0, 64.0, 96.0))])
            .build(&first, &mut ids)
            .unwrap();
        assert_eq!(material_facing(&second, DVec3::NEG_Z), bottom);
        assert_eq!(
            material_facing(&second, DVec3::NEG_X),
            material_facing(&solid, DVec3::NEG_X)
        );
    }

    #[test]
    fn pulling_a_corner_inside_is_refused() {
        let (solid, mut ids) = cube();
        let mesh = BrushMesh::from_solid(&solid).unwrap();
        let top = vertex(&mesh, DVec3::splat(64.0));
        let result = mesh.moved(&[(top, DVec3::splat(32.0))]).build(&solid, &mut ids);
        assert_eq!(result.unwrap_err(), VertexError::NotConvex);
    }

    #[test]
    fn a_corner_dragged_onto_another_merges_with_it() {
        let (solid, mut ids) = cube();
        let mesh = BrushMesh::from_solid(&solid).unwrap();
        let top = vertex(&mesh, DVec3::splat(64.0));
        let target = DVec3::new(0.0, 64.0, 64.0);
        let moved = mesh.moved(&[(top, target)]);
        assert!(moved.overlaps(&[top]));
        let (built, rebuilt) = moved.build(&solid, &mut ids).unwrap();
        assert_eq!(rebuilt.points.len(), 7);
        assert_eq!(corners(&built).len(), 7);
    }

    #[test]
    fn a_split_face_bends_along_the_split() {
        let (solid, mut ids) = cube();
        let mesh = BrushMesh::from_solid(&solid).unwrap();
        let a = vertex(&mesh, DVec3::new(0.0, 0.0, 64.0));
        let b = vertex(&mesh, DVec3::new(64.0, 64.0, 64.0));
        let split = mesh.split_face(a, b).unwrap();
        assert_eq!(split.faces.len(), 7);
        let (unchanged, _) = split.build(&solid, &mut ids).unwrap();
        assert_eq!(unchanged.sides().count(), 6);
        let raised = split.moved(&[(a, DVec3::new(0.0, 0.0, 96.0)), (b, DVec3::new(64.0, 64.0, 96.0))]);
        let (built, _) = raised.build(&solid, &mut ids).unwrap();
        assert_eq!(built.sides().count(), 7);
        assert_eq!(corners(&built).len(), 8);
    }

    #[test]
    fn splitting_needs_two_corners_of_one_face_without_an_edge_between() {
        let (solid, _) = cube();
        let mesh = BrushMesh::from_solid(&solid).unwrap();
        let a = vertex(&mesh, DVec3::ZERO);
        let b = vertex(&mesh, DVec3::new(64.0, 0.0, 0.0));
        let c = vertex(&mesh, DVec3::splat(64.0));
        assert_eq!(mesh.split_face(a, b).unwrap_err(), VertexError::AlreadyJoined);
        assert_eq!(mesh.split_face(a, c).unwrap_err(), VertexError::NotOnOneFace);
    }
}

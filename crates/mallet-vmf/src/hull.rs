//! Convex hulls of point sets, and splitting a triangle mesh into nearly convex parts.
//!
//! Brushes are convex, so turning a prop's model into brushes means finding convex shapes
//! that cover it: either the collision model's pieces, which are convex already, or parts
//! of the visual mesh whose hulls stay close to its surface.

use std::collections::{HashMap, HashSet};

use glam::DVec3;

use crate::geometry::Plane;

/// Points closer than this are treated as one.
const WELD_DISTANCE: f64 = 0.01;

/// How far outside a hull a point must be before it grows the hull.
const HULL_EPSILON: f64 = 0.001;

/// The thinnest a hull can be before it counts as flat.
const MIN_THICKNESS: f64 = 0.01;

/// Faces this far apart in angle are never merged into one brush side.
const MAX_MERGE_COSINE: f64 = 0.866;

/// How many times a mesh part may be split in half before it is used as it is.
const MAX_SPLIT_DEPTH: usize = 12;

/// A convex hull: the points on it and its triangles, wound counter-clockwise when seen
/// from outside.
#[derive(Debug, Clone, PartialEq)]
pub struct Hull {
    pub points: Vec<DVec3>,
    pub triangles: Vec<[usize; 3]>,
    /// The outward plane of each triangle.
    pub planes: Vec<Plane>,
}

impl Hull {
    /// The volume enclosed by the hull.
    pub fn volume(&self) -> f64 {
        let center = self.center();
        self.triangles
            .iter()
            .map(|&[a, b, c]| {
                let [a, b, c] = [a, b, c].map(|index| self.points[index] - center);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    /// The average of the hull's points, which lies inside it.
    pub fn center(&self) -> DVec3 {
        self.points.iter().copied().sum::<DVec3>() / self.points.len().max(1) as f64
    }

    /// How far inside the hull a point lies: zero on its surface, larger further in.
    pub fn depth(&self, point: DVec3) -> f64 {
        self.planes
            .iter()
            .map(|plane| -plane.distance(point))
            .fold(f64::INFINITY, f64::min)
            .max(0.0)
    }

    /// Planes for a brush covering the hull, with at most `max_sides` sides.
    ///
    /// Neighbouring triangles that are nearly coplanar share one plane, so a curved surface
    /// becomes a few larger sides instead of a side per triangle. Each plane is pushed out
    /// until every corner of its triangles lies behind it, so the brush never cuts into the
    /// hull; it grows by at most the flatness allowed, which starts tiny and doubles until
    /// the sides fit.
    pub fn brush_planes(&self, max_sides: usize) -> Vec<Plane> {
        let mut flatness = WELD_DISTANCE;
        loop {
            let planes = self.merged_planes(flatness);
            if planes.len() <= max_sides.max(4) || flatness > 1.0e4 {
                return planes;
            }
            flatness *= 2.0;
        }
    }

    fn merged_planes(&self, flatness: f64) -> Vec<Plane> {
        let mut order: Vec<usize> = (0..self.triangles.len()).collect();
        let area = |index: usize| {
            let [a, b, c] = self.triangles[index].map(|point| self.points[point]);
            (b - a).cross(c - a).length()
        };
        order.sort_by(|&a, &b| area(b).total_cmp(&area(a)));
        let mut groups: Vec<(Plane, f64)> = Vec::new();
        for index in order {
            let plane = self.planes[index];
            let corners = self.triangles[index].map(|point| self.points[point]);
            let joined = groups.iter_mut().find(|(seed, _)| {
                seed.normal.dot(plane.normal) >= MAX_MERGE_COSINE
                    && corners.iter().all(|corner| seed.distance(*corner).abs() <= flatness)
            });
            let reach = corners
                .iter()
                .map(|corner| plane.normal.dot(*corner))
                .fold(f64::NEG_INFINITY, f64::max);
            match joined {
                Some((seed, dist)) => {
                    let reach = corners
                        .iter()
                        .map(|corner| seed.normal.dot(*corner))
                        .fold(f64::NEG_INFINITY, f64::max);
                    *dist = dist.max(reach);
                }
                None => groups.push((plane, reach)),
            }
        }
        groups
            .into_iter()
            .map(|(seed, dist)| Plane {
                normal: seed.normal,
                dist,
            })
            .collect()
    }
}

/// The convex hull of a set of points, or `None` when they are flat, on a line or fewer
/// than four.
pub fn convex_hull(points: &[DVec3]) -> Option<Hull> {
    let points = weld(points);
    if points.len() < 4 {
        return None;
    }
    let first = (0..points.len()).min_by(|&a, &b| points[a].x.total_cmp(&points[b].x))?;
    let second = farthest(&points, |point| point.distance(points[first]))?;
    let line = (points[second] - points[first]).normalize_or_zero();
    let third = farthest(&points, |point| {
        let offset = point - points[first];
        (offset - line * offset.dot(line)).length()
    })?;
    let across = points[third] - points[first];
    if (across - line * across.dot(line)).length() <= MIN_THICKNESS {
        return None;
    }
    let base = Plane::from_points([points[first], points[second], points[third]])?;
    let fourth = farthest(&points, |point| base.distance(point).abs())?;
    if base.distance(points[fourth]).abs() <= MIN_THICKNESS {
        return None;
    }

    let mut triangles = vec![
        [first, second, third],
        [first, fourth, second],
        [second, fourth, third],
        [third, fourth, first],
    ];
    let inside = (points[first] + points[second] + points[third] + points[fourth]) / 4.0;
    for triangle in &mut triangles {
        let plane = triangle_plane(&points, *triangle, DVec3::Z);
        if plane.distance(inside) > 0.0 {
            triangle.swap(1, 2);
        }
    }
    let mut planes: Vec<Plane> = triangles
        .iter()
        .map(|triangle| triangle_plane(&points, *triangle, DVec3::Z))
        .collect();

    let mut order: Vec<usize> = (0..points.len())
        .filter(|index| ![first, second, third, fourth].contains(index))
        .collect();
    order.sort_by(|&a, &b| points[b].distance(inside).total_cmp(&points[a].distance(inside)));
    for index in order {
        let point = points[index];
        let visible: Vec<bool> = planes
            .iter()
            .map(|plane| plane.distance(point) > HULL_EPSILON)
            .collect();
        if !visible.contains(&true) {
            continue;
        }
        let mut edges: HashMap<(usize, usize), DVec3> = HashMap::new();
        let mut kept_triangles = Vec::with_capacity(triangles.len());
        let mut kept_planes = Vec::with_capacity(planes.len());
        for ((triangle, plane), seen) in triangles.iter().zip(&planes).zip(&visible) {
            if *seen {
                for edge in 0..3 {
                    edges.insert((triangle[edge], triangle[(edge + 1) % 3]), plane.normal);
                }
            } else {
                kept_triangles.push(*triangle);
                kept_planes.push(*plane);
            }
        }
        for (&(a, b), normal) in &edges {
            if !edges.contains_key(&(b, a)) {
                let triangle = [a, b, index];
                kept_triangles.push(triangle);
                kept_planes.push(triangle_plane(&points, triangle, *normal));
            }
        }
        triangles = kept_triangles;
        planes = kept_planes;
    }

    let used: HashSet<usize> = triangles.iter().flatten().copied().collect();
    let mut remap = HashMap::new();
    let mut kept_points = Vec::new();
    for (index, point) in points.iter().enumerate() {
        if used.contains(&index) {
            remap.insert(index, kept_points.len());
            kept_points.push(*point);
        }
    }
    let triangles = triangles
        .into_iter()
        .map(|triangle| triangle.map(|index| remap[&index]))
        .collect();
    Some(Hull {
        points: kept_points,
        triangles,
        planes,
    })
}

/// Like [`convex_hull`], but gives flat point sets a back face `thickness` behind them, so
/// a single-sided panel such as a sign still becomes a solid. `facing` picks which side of
/// the panel is its front. Also says whether the back face was added.
fn solid_hull(points: &[DVec3], facing: DVec3, thickness: f64) -> Option<(Hull, bool)> {
    if let Some(hull) = convex_hull(points) {
        return Some((hull, false));
    }
    let normal = facing.normalize_or_zero();
    if normal == DVec3::ZERO {
        return None;
    }
    let mut thick: Vec<DVec3> = points.to_vec();
    thick.extend(points.iter().map(|point| *point - normal * thickness));
    convex_hull(&thick).map(|hull| (hull, true))
}

/// Splits a triangle mesh into parts whose convex hulls stay within `tolerance` units of
/// the mesh, and returns those hulls; at most `max_parts` of them.
///
/// Separate pieces of the mesh, such as the bars of a railing, are handled one by one. A
/// piece whose hull swallows points deeper than `tolerance`, like the inside of an L-shaped
/// bracket, is cut in two by an axis-aligned plane, picking the cut that leaves the
/// smallest hulls, and each half is checked again. Flat pieces get `thickness` units of
/// depth behind their front.
pub fn convex_parts(
    positions: &[DVec3],
    triangles: &[[u32; 3]],
    tolerance: f64,
    thickness: f64,
    max_parts: usize,
) -> Vec<Hull> {
    let (points, triangles) = weld_mesh(positions, triangles);
    let mut parts: Vec<Part> = connected_pieces(points.len(), &triangles)
        .into_iter()
        .filter_map(|piece| Part::new(&points, piece, thickness, 0))
        .collect();
    while parts.len() < max_parts.max(1) {
        let worst = parts
            .iter()
            .enumerate()
            .filter(|(_, part)| part.can_split(tolerance))
            .max_by(|(_, a), (_, b)| a.concavity.total_cmp(&b.concavity))
            .map(|(index, _)| index);
        let Some(index) = worst else { break };
        match parts[index].split(&points, thickness) {
            Some((front, back)) => {
                parts[index] = front;
                parts.push(back);
            }
            None => parts[index].depth = MAX_SPLIT_DEPTH,
        }
    }
    parts.into_iter().map(|part| part.hull).collect()
}

/// A set of triangles from a welded mesh with the hull around them.
struct Part {
    triangles: Vec<[usize; 3]>,
    hull: Hull,
    /// How far the hull strays from the triangles: the deeper of how far inside it a
    /// corner or triangle centre lies, and how far its surface is from the nearest triangle.
    concavity: f64,
    depth: usize,
}

impl Part {
    fn new(points: &[DVec3], triangles: Vec<[usize; 3]>, thickness: f64, depth: usize) -> Option<Self> {
        let mut corners: Vec<DVec3> = triangles.iter().flatten().map(|index| points[*index]).collect();
        corners.sort_by(|a, b| {
            a.to_array()
                .partial_cmp(&b.to_array())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        corners.dedup();
        let facing = triangles
            .iter()
            .map(|&[a, b, c]| (points[b] - points[a]).cross(points[c] - points[a]))
            .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))?;
        let (hull, thickened) = solid_hull(&corners, facing, thickness)?;
        let centres = triangles
            .iter()
            .map(|&[a, b, c]| (points[a] + points[b] + points[c]) / 3.0);
        let depth_inside = corners
            .iter()
            .copied()
            .chain(centres)
            .map(|point| hull.depth(point))
            .fold(0.0, f64::max);
        let added = if thickened { thickness } else { 0.0 };
        let concavity = depth_inside.max(surface_gap(&hull, points, &triangles) - added);
        Some(Self {
            triangles,
            hull,
            concavity,
            depth,
        })
    }

    fn can_split(&self, tolerance: f64) -> bool {
        self.concavity > tolerance && self.depth < MAX_SPLIT_DEPTH && self.triangles.len() > 1
    }

    /// Cuts the part in two along whichever of a few axis-aligned planes leaves the
    /// smallest total hull volume. Triangles go to the side their centre is on.
    fn split(&self, points: &[DVec3], thickness: f64) -> Option<(Part, Part)> {
        let centres: Vec<DVec3> = self
            .triangles
            .iter()
            .map(|&[a, b, c]| (points[a] + points[b] + points[c]) / 3.0)
            .collect();
        let mut best: Option<(f64, Part, Part)> = None;
        for axis in 0..3 {
            let mut values: Vec<f64> = centres.iter().map(|centre| centre[axis]).collect();
            values.sort_by(f64::total_cmp);
            for share in [0.25, 0.5, 0.75] {
                let cut = values[((values.len() - 1) as f64 * share) as usize];
                let (front, back): (Vec<_>, Vec<_>) = self
                    .triangles
                    .iter()
                    .zip(&centres)
                    .partition(|(_, centre)| centre[axis] <= cut);
                if front.is_empty() || back.is_empty() {
                    continue;
                }
                let make = |side: Vec<(&[usize; 3], &DVec3)>| {
                    let triangles = side.into_iter().map(|(triangle, _)| *triangle).collect();
                    Part::new(points, triangles, thickness, self.depth + 1)
                };
                let (Some(front), Some(back)) = (make(front), make(back)) else {
                    continue;
                };
                let volume = front.hull.volume() + back.hull.volume();
                if best.as_ref().is_none_or(|(least, _, _)| volume < *least) {
                    best = Some((volume, front, back));
                }
            }
        }
        best.map(|(_, front, back)| (front, back))
    }
}

/// The largest distance from a point on the hull's surface to the nearest of the
/// triangles. A hull wrapped tightly round its triangles has a small gap; one spanning the
/// open side of a thin L-shape has a gap as wide as the opening.
fn surface_gap(hull: &Hull, points: &[DVec3], triangles: &[[usize; 3]]) -> f64 {
    let shapes: Vec<([DVec3; 3], DVec3, DVec3)> = triangles
        .iter()
        .map(|triangle| {
            let corners = triangle.map(|index| points[index]);
            let min = corners[0].min(corners[1]).min(corners[2]);
            let max = corners[0].max(corners[1]).max(corners[2]);
            (corners, min, max)
        })
        .collect();
    let samples = hull.triangles.iter().flat_map(|triangle| {
        let [a, b, c] = triangle.map(|index| hull.points[index]);
        [
            (a + b + c) / 3.0,
            (a * 4.0 + b + c) / 6.0,
            (a + b * 4.0 + c) / 6.0,
            (a + b + c * 4.0) / 6.0,
        ]
    });
    let mut gap: f64 = 0.0;
    for sample in samples {
        let mut nearest = f64::INFINITY;
        for (corners, min, max) in &shapes {
            let outside = (*min - sample).max(sample - *max).max(DVec3::ZERO);
            if outside.length_squared() >= nearest * nearest {
                continue;
            }
            nearest = nearest.min(sample.distance(closest_on_triangle(sample, *corners)));
            if nearest <= gap {
                break;
            }
        }
        gap = gap.max(nearest);
    }
    gap
}

/// The point of a triangle closest to `point`.
fn closest_on_triangle(point: DVec3, [a, b, c]: [DVec3; 3]) -> DVec3 {
    let (ab, ac, ap) = (b - a, c - a, point - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = point - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = point - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denominator = 1.0 / (va + vb + vc);
    a + ab * (vb * denominator) + ac * (vc * denominator)
}

/// The outward plane of a triangle, or `fallback` as its normal when it has no area.
fn triangle_plane(points: &[DVec3], [a, b, c]: [usize; 3], fallback: DVec3) -> Plane {
    let normal = (points[b] - points[a]).cross(points[c] - points[a]);
    let normal = if normal.length() > 1e-12 {
        normal.normalize()
    } else {
        fallback
    };
    Plane {
        normal,
        dist: normal.dot(points[a]),
    }
}

/// The index of the point that scores highest, or `None` for no points.
fn farthest(points: &[DVec3], score: impl Fn(DVec3) -> f64) -> Option<usize> {
    (0..points.len()).max_by(|&a, &b| score(points[a]).total_cmp(&score(points[b])))
}

/// The cell a point falls in when points within [`WELD_DISTANCE`] are merged.
fn weld_key(point: DVec3) -> [i64; 3] {
    (point / WELD_DISTANCE).round().to_array().map(|value| value as i64)
}

/// The points with near-duplicates removed.
fn weld(points: &[DVec3]) -> Vec<DVec3> {
    let mut seen = HashSet::new();
    points
        .iter()
        .copied()
        .filter(|point| point.is_finite() && seen.insert(weld_key(*point)))
        .collect()
}

/// Merges coincident vertices so triangles that touch share indices, and drops triangles
/// without area.
fn weld_mesh(positions: &[DVec3], triangles: &[[u32; 3]]) -> (Vec<DVec3>, Vec<[usize; 3]>) {
    let mut points = Vec::new();
    let mut cells: HashMap<[i64; 3], usize> = HashMap::new();
    let mut remap = Vec::with_capacity(positions.len());
    for point in positions {
        let index = *cells.entry(weld_key(*point)).or_insert_with(|| {
            points.push(*point);
            points.len() - 1
        });
        remap.push(index);
    }
    let triangles = triangles
        .iter()
        .filter_map(|triangle| {
            let [a, b, c] = triangle.map(|index| remap.get(index as usize).copied());
            let triangle = [a?, b?, c?];
            let [pa, pb, pc] = triangle.map(|index| points[index]);
            ((pb - pa).cross(pc - pa).length() > 1e-9).then_some(triangle)
        })
        .collect();
    (points, triangles)
}

/// Groups triangles into pieces that are connected through shared corners.
fn connected_pieces(point_count: usize, triangles: &[[usize; 3]]) -> Vec<Vec<[usize; 3]>> {
    let mut parent: Vec<usize> = (0..point_count).collect();
    fn root(parent: &mut [usize], mut index: usize) -> usize {
        while parent[index] != index {
            parent[index] = parent[parent[index]];
            index = parent[index];
        }
        index
    }
    for &[a, b, c] in triangles {
        let ra = root(&mut parent, a);
        for other in [b, c] {
            let ro = root(&mut parent, other);
            parent[ro] = ra;
        }
    }
    let mut pieces: HashMap<usize, Vec<[usize; 3]>> = HashMap::new();
    for triangle in triangles {
        let key = root(&mut parent, triangle[0]);
        pieces.entry(key).or_default().push(*triangle);
    }
    let mut pieces: Vec<Vec<[usize; 3]>> = pieces.into_values().collect();
    pieces.sort_by_key(|piece| std::cmp::Reverse(piece.len()));
    pieces
}

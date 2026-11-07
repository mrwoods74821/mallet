//! Where VBSP puts the luxels of a face's lightmap, and the points VRAD lights them at.
//!
//! A face's lightmap runs along its texture axes with one luxel every `lightmapscale`
//! units: luxel `(i, j)` sits where `p · s = s_min + i` and `p · t = t_min + j`, with `s`
//! and `t` the unit texture axes divided by the scale, and `s_min` and `t_min` the face's
//! lowest coordinates rounded down. A face `w` luxels across has `w + 1` luxels along that
//! side, one on each edge. Luxels whose point lies outside the face light the nearest
//! point inside it instead, as VRAD moves them, so light does not bleed in from behind the
//! brushes next to it.

use glam::{DMat3, DVec2, DVec3};

/// How far inside a face, in luxels, a luxel that falls outside it is moved.
const INSET: f64 = 0.25;

/// The luxel grid of one face.
#[derive(Clone, Debug, PartialEq)]
pub struct LuxelGrid {
    /// Luxels per unit along the grid's two directions.
    pub s: DVec3,
    pub t: DVec3,
    /// The grid coordinates of luxel `(0, 0)`.
    pub s_min: f64,
    pub t_min: f64,
    /// Luxels along each direction.
    pub width: u32,
    pub height: u32,
    /// Units per luxel, which can be coarser than asked for a face too big to fit.
    pub scale: f64,
    /// From grid coordinates and the plane's distance back to a point.
    to_world: DMat3,
    distance: f64,
    /// The face's corners in grid coordinates.
    corners: Vec<DVec2>,
    middle: DVec2,
}

impl LuxelGrid {
    /// The grid of a convex face with `corners` in winding order, its texture axes and
    /// `scale` units per luxel. A face more than `max_side` luxels along a side gets
    /// coarser luxels so it fits. When the texture axes do not span the face, the grid
    /// runs along two directions in its plane instead. `None` for a face with fewer than
    /// three corners or no normal.
    pub fn new(corners: &[DVec3], normal: DVec3, axes: [DVec3; 2], scale: f64, max_side: u32) -> Option<Self> {
        let normal = normal.try_normalize()?;
        if corners.len() < 3 || max_side < 2 {
            return None;
        }
        let [mut u, mut v] = axes.map(|axis| axis.normalize_or_zero());
        if u.cross(v).dot(normal).abs() < 0.01 {
            u = normal.any_orthonormal_vector();
            v = normal.cross(u);
        }
        let mut scale = if scale.is_finite() && scale > 0.0 { scale } else { 16.0 };
        loop {
            let (s, t) = (u / scale, v / scale);
            let flat: Vec<DVec2> = corners
                .iter()
                .map(|point| DVec2::new(point.dot(s), point.dot(t)))
                .collect();
            let low = flat.iter().fold(DVec2::INFINITY, |low, point| low.min(*point));
            let high = flat.iter().fold(DVec2::NEG_INFINITY, |high, point| high.max(*point));
            let (s_min, t_min) = (low.x.floor(), low.y.floor());
            let width = (high.x.ceil() - s_min) as u32 + 1;
            let height = (high.y.ceil() - t_min) as u32 + 1;
            if width <= max_side && height <= max_side {
                let middle = flat.iter().sum::<DVec2>() / flat.len() as f64;
                return Some(Self {
                    s,
                    t,
                    s_min,
                    t_min,
                    width,
                    height,
                    scale,
                    to_world: DMat3::from_cols(s, t, normal).transpose().inverse(),
                    distance: normal.dot(corners[0]),
                    corners: flat,
                    middle,
                });
            }
            scale *= (f64::from(width.max(height)) / f64::from(max_side - 1)).max(1.01);
        }
    }

    /// The point luxel `(i, j)` lights: its place on the face, or the nearest point a
    /// little inside the face when its place is outside it.
    pub fn point(&self, i: u32, j: u32) -> DVec3 {
        let at = DVec2::new(self.s_min + f64::from(i), self.t_min + f64::from(j));
        let inside = if self.contains(at) { at } else { self.pull_inside(at) };
        self.to_world * DVec3::new(inside.x, inside.y, self.distance)
    }

    fn contains(&self, at: DVec2) -> bool {
        let mut sign = 0.0f64;
        for (index, a) in self.corners.iter().enumerate() {
            let b = self.corners[(index + 1) % self.corners.len()];
            let cross = (b - *a).perp_dot(at - *a);
            if cross.abs() < 1e-9 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
        true
    }

    /// The point of the face's outline nearest `at`, moved [`INSET`] luxels towards the
    /// middle of the face, but never past it.
    fn pull_inside(&self, at: DVec2) -> DVec2 {
        let mut nearest = self.middle;
        let mut best = f64::INFINITY;
        for (index, a) in self.corners.iter().enumerate() {
            let b = self.corners[(index + 1) % self.corners.len()];
            let edge = b - *a;
            let along = (at - *a).dot(edge) / edge.length_squared().max(1e-12);
            let point = *a + edge * along.clamp(0.0, 1.0);
            let distance = point.distance_squared(at);
            if distance < best {
                best = distance;
                nearest = point;
            }
        }
        let toward = self.middle - nearest;
        let length = toward.length();
        if length <= INSET {
            self.middle
        } else {
            nearest + toward / length * INSET
        }
    }
}

/// Packs rectangles of `(width, height)` into rows of a strip `strip` wide, tallest
/// first, with `gap` between them. Returns each rectangle's top left corner in the order
/// given, `None` for one wider than the strip, and the height the rows take.
pub fn pack_rows(sizes: &[(u32, u32)], strip: u32, gap: u32) -> (Vec<Option<(u32, u32)>>, u32) {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by(|a, b| sizes[*b].1.cmp(&sizes[*a].1).then(sizes[*b].0.cmp(&sizes[*a].0)));
    let mut places = vec![None; sizes.len()];
    let (mut x, mut y, mut row) = (0, 0, 0);
    for index in order {
        let (width, height) = sizes[index];
        if width > strip {
            continue;
        }
        if x + width > strip {
            y += row + gap;
            x = 0;
            row = 0;
        }
        places[index] = Some((x, y));
        x += width + gap;
        row = row.max(height);
    }
    (places, y + row)
}

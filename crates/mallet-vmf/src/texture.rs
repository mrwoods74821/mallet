//! Texture alignment on brush faces, as Hammer's Face Edit sheet does it: world and face
//! alignment, rotation about the face, and justifying or fitting a texture to a face.
//!
//! Source maps texture coordinates as `u = point · uaxis / scale + offset` (in texels), and
//! the same for `v`. Rotations turn both axes about the face normal; justify and fit
//! change the scale and offset so the face's edges land where they should on the texture.

use glam::DVec3;

use crate::{Side, TexAxis};

/// Where to line a texture up on a face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Justify {
    Left,
    Right,
    Top,
    Bottom,
    Center,
    /// Stretch the texture once across the face.
    Fit,
}

/// The axes Hammer and the compilers pick for a face aligned to the world: the major axis
/// closest to the normal decides which world plane the texture is projected from.
pub fn world_axes(normal: DVec3) -> (DVec3, DVec3) {
    // Quake's base axes, in the order Hammer tests them: floor, ceiling, west, east,
    // south, north.
    const BASES: [(DVec3, DVec3, DVec3); 6] = [
        (DVec3::Z, DVec3::X, DVec3::NEG_Y),
        (DVec3::NEG_Z, DVec3::X, DVec3::NEG_Y),
        (DVec3::X, DVec3::Y, DVec3::NEG_Z),
        (DVec3::NEG_X, DVec3::Y, DVec3::NEG_Z),
        (DVec3::Y, DVec3::X, DVec3::NEG_Z),
        (DVec3::NEG_Y, DVec3::X, DVec3::NEG_Z),
    ];
    let mut best = (f64::MIN, DVec3::X, DVec3::NEG_Y);
    for (base, u, v) in BASES {
        let dot = normal.dot(base);
        if dot > best.0 + 1e-9 {
            best = (dot, u, v);
        }
    }
    (best.1, best.2)
}

/// Axes lying in the face itself, so the texture is not stretched on slopes: `v` points
/// down the face, and floors and ceilings fall back to world alignment.
pub fn face_axes(normal: DVec3) -> (DVec3, DVec3) {
    let normal = normal.normalize_or_zero();
    let down = DVec3::NEG_Z - normal * normal.dot(DVec3::NEG_Z);
    if down.length() < 1e-6 {
        return world_axes(normal);
    }
    let v = down.normalize();
    let u = normal.cross(v).normalize();
    (u, v)
}

/// Turns a vector about `axis` by `degrees`, counter-clockwise looking against the axis.
fn rotate(vector: DVec3, axis: DVec3, degrees: f64) -> DVec3 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let axis = axis.normalize_or_zero();
    vector * cos + axis.cross(vector) * sin + axis * axis.dot(vector) * (1.0 - cos)
}

/// The face's normal from its plane points, pointing out of the brush.
pub fn side_normal(side: &Side) -> DVec3 {
    let [a, b, c] = side.plane;
    (c - a).cross(b - a).normalize_or_zero()
}

/// The range of `point · axis` over the face, in world units.
fn extent(axis: DVec3, points: &[DVec3]) -> (f64, f64) {
    points
        .iter()
        .map(|point| point.dot(axis))
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), value| {
            (min.min(value), max.max(value))
        })
}

/// Keeps an offset within one texture repeat, as Hammer shows it.
fn wrap(offset: f64, size: f64) -> f64 {
    if size <= 0.0 {
        return offset;
    }
    let wrapped = offset.rem_euclid(size);
    if (wrapped - size).abs() < 1e-6 {
        0.0
    } else {
        (wrapped * 1000.0).round() / 1000.0
    }
}

impl Side {
    /// Aligns the texture to the world or to the face, keeping scale and shift and
    /// applying the side's rotation again.
    pub fn align(&mut self, to_face: bool) {
        let normal = side_normal(self);
        let (u, v) = if to_face { face_axes(normal) } else { world_axes(normal) };
        self.uaxis.axis = u;
        self.vaxis.axis = v;
        let rotation = self.rotation.unwrap_or(0.0);
        if rotation != 0.0 {
            self.uaxis.axis = rotate(u, normal, rotation);
            self.vaxis.axis = rotate(v, normal, rotation);
        }
    }

    /// Sets the texture rotation in degrees, turning both axes about the face normal by
    /// the difference from the current rotation.
    pub fn set_rotation(&mut self, degrees: f64) {
        let delta = degrees - self.rotation.unwrap_or(0.0);
        if delta != 0.0 {
            let normal = side_normal(self);
            self.uaxis.axis = rotate(self.uaxis.axis, normal, delta);
            self.vaxis.axis = rotate(self.vaxis.axis, normal, delta);
        }
        self.rotation = Some(degrees);
    }

    /// Lines the texture up on `points` (the face polygon, or several faces' points for
    /// "treat as one"). `size` is the texture size in texels.
    pub fn justify(&mut self, how: Justify, points: &[DVec3], size: [f64; 2]) {
        if points.is_empty() {
            return;
        }
        let [width, height] = size.map(|side| side.max(1.0));
        let (u_min, u_max) = extent(self.uaxis.axis, points);
        let (v_min, v_max) = extent(self.vaxis.axis, points);
        if how == Justify::Fit {
            self.uaxis.scale = ((u_max - u_min) / width).max(1e-4);
            self.vaxis.scale = ((v_max - v_min) / height).max(1e-4);
        }
        let (u, v) = (&mut self.uaxis, &mut self.vaxis);
        // The texel a world position lands on is `position / scale + offset`.
        let start = |axis: &TexAxis, min: f64| -min / axis.scale;
        let end = |axis: &TexAxis, max: f64, size: f64| size - max / axis.scale;
        let middle = |axis: &TexAxis, min: f64, max: f64, size: f64| size / 2.0 - (min + max) / 2.0 / axis.scale;
        match how {
            Justify::Left => u.offset = start(u, u_min),
            Justify::Right => u.offset = end(u, u_max, width),
            Justify::Top => v.offset = start(v, v_min),
            Justify::Bottom => v.offset = end(v, v_max, height),
            Justify::Center => {
                u.offset = middle(u, u_min, u_max, width);
                v.offset = middle(v, v_min, v_max, height);
            }
            Justify::Fit => {
                u.offset = start(u, u_min);
                v.offset = start(v, v_min);
            }
        }
        u.offset = wrap(u.offset, width);
        v.offset = wrap(v.offset, height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vmf;

    /// A 128 x 64 x 32 box's side whose normal points along `normal`.
    fn side_facing(normal: DVec3) -> Side {
        let text = r#"world { "id" "1" "classname" "worldspawn"
solid { "id" "2"
side { "id" "1" "plane" "(0 64 32) (128 64 32) (128 0 32)" "material" "A" "uaxis" "[1 0 0 0] 0.25" "vaxis" "[0 -1 0 0] 0.25" }
side { "id" "2" "plane" "(0 0 0) (128 0 0) (128 64 0)" "material" "A" "uaxis" "[1 0 0 0] 0.25" "vaxis" "[0 -1 0 0] 0.25" }
side { "id" "3" "plane" "(0 64 32) (0 0 32) (0 0 0)" "material" "A" "uaxis" "[0 1 0 0] 0.25" "vaxis" "[0 0 -1 0] 0.25" }
side { "id" "4" "plane" "(128 64 0) (128 0 0) (128 0 32)" "material" "A" "uaxis" "[0 1 0 0] 0.25" "vaxis" "[0 0 -1 0] 0.25" }
side { "id" "5" "plane" "(128 64 32) (0 64 32) (0 64 0)" "material" "A" "uaxis" "[1 0 0 0] 0.25" "vaxis" "[0 0 -1 0] 0.25" }
side { "id" "6" "plane" "(128 0 0) (0 0 0) (0 0 32)" "material" "A" "uaxis" "[1 0 0 0] 0.25" "vaxis" "[0 0 -1 0] 0.25" }
} }"#;
        let vmf = Vmf::parse(text).unwrap();
        let world = vmf.world().unwrap();
        let solid = world.solids().next().unwrap();
        let side = solid
            .sides()
            .find(|side| side_normal(side).dot(normal) > 0.99)
            .cloned()
            .unwrap();
        side
    }

    #[test]
    fn plane_points_give_outward_normals() {
        assert!(side_normal(&side_facing(DVec3::Z)).abs_diff_eq(DVec3::Z, 1e-9));
        assert!(side_normal(&side_facing(DVec3::NEG_X)).abs_diff_eq(DVec3::NEG_X, 1e-9));
    }

    #[test]
    fn world_alignment_matches_hammer() {
        assert_eq!(world_axes(DVec3::Z), (DVec3::X, DVec3::NEG_Y));
        assert_eq!(world_axes(DVec3::X), (DVec3::Y, DVec3::NEG_Z));
        assert_eq!(world_axes(DVec3::NEG_Y), (DVec3::X, DVec3::NEG_Z));
        // A 45 degree slope takes the floor's axes, as Hammer breaks ties.
        assert_eq!(world_axes(DVec3::new(1.0, 0.0, 1.0).normalize()).0, DVec3::X);
    }

    #[test]
    fn face_alignment_follows_slopes() {
        let normal = DVec3::new(1.0, 0.0, 1.0).normalize();
        let (u, v) = face_axes(normal);
        assert!(u.dot(normal).abs() < 1e-9 && v.dot(normal).abs() < 1e-9);
        assert!(v.z < 0.0, "v points down the slope");
        assert!(u.abs_diff_eq(DVec3::Y, 1e-9));
    }

    #[test]
    fn rotation_turns_both_axes_and_round_trips() {
        let mut side = side_facing(DVec3::Z);
        side.set_rotation(90.0);
        assert!(side.uaxis.axis.abs_diff_eq(DVec3::Y, 1e-9), "{:?}", side.uaxis.axis);
        assert!(side.vaxis.axis.abs_diff_eq(DVec3::X, 1e-9), "{:?}", side.vaxis.axis);
        side.set_rotation(0.0);
        assert!(side.uaxis.axis.abs_diff_eq(DVec3::X, 1e-9));
        side.align(false);
        assert!(side.uaxis.axis.abs_diff_eq(DVec3::X, 1e-9));
    }

    #[test]
    fn fit_and_justify_land_edges_on_texture_edges() {
        let mut side = side_facing(DVec3::Z);
        let points = [
            DVec3::new(0.0, 0.0, 32.0),
            DVec3::new(128.0, 0.0, 32.0),
            DVec3::new(128.0, 64.0, 32.0),
            DVec3::new(0.0, 64.0, 32.0),
        ];
        let texel = |side: &Side, point: DVec3| {
            [
                point.dot(side.uaxis.axis) / side.uaxis.scale + side.uaxis.offset,
                point.dot(side.vaxis.axis) / side.vaxis.scale + side.vaxis.offset,
            ]
        };
        side.justify(Justify::Fit, &points, [256.0, 256.0]);
        assert!((side.uaxis.scale - 0.5).abs() < 1e-9);
        assert!((side.vaxis.scale - 0.25).abs() < 1e-9);
        let corner = texel(&side, DVec3::new(0.0, 64.0, 32.0));
        assert!(
            corner[0].abs() < 1e-6 && corner[1].rem_euclid(256.0) < 1e-6,
            "{corner:?}"
        );

        let mut side = side_facing(DVec3::Z);
        side.justify(Justify::Right, &points, [128.0, 128.0]);
        let right = texel(&side, DVec3::new(128.0, 0.0, 32.0))[0];
        assert!(
            (right.rem_euclid(128.0)).abs() < 1e-6,
            "the right edge is at a texture edge: {right}"
        );
        side.justify(Justify::Center, &points, [128.0, 128.0]);
        let middle = texel(&side, DVec3::new(64.0, 32.0, 32.0));
        assert!((middle[0].rem_euclid(128.0) - 64.0).abs() < 1e-6, "{middle:?}");
    }
}

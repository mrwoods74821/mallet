//! Affine transforms of brushes and entity angles: rotating, scaling, skewing and
//! mirroring, with Hammer's texture lock.
//!
//! Every transform is a [`DMat4`] holding a linear part and a translation. Plane points
//! go through the whole matrix; texture axes, displacement normals and entity angles go
//! through the linear part only.

use glam::{DMat3, DMat4, DVec3};

use crate::kv::{Block, Item};
use crate::{parse_numbers, Side, Solid};

/// Values this close to a whole number become that number, so a 90 degree turn leaves
/// `64` rather than `63.99999999999999`.
const SNAP: f64 = 1e-6;

/// How textures follow a transformed brush.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextureLock {
    /// Texture axes stay as they are, so textures keep their world alignment.
    Off,
    /// Textures turn and move with the brush but keep their scale, pinned at `anchor`:
    /// Hammer's texture lock without texture scale lock.
    Align {
        /// The point whose texture coordinates stay the same after the transform.
        anchor: DVec3,
    },
    /// Textures follow the brush exactly, stretching with it: Hammer's texture lock with
    /// texture scale lock.
    Full,
}

/// Rounds away floating-point noise: values within a millionth of a whole number snap to
/// it, others keep six decimals, and negative zero becomes zero.
pub fn clean(value: f64) -> f64 {
    let rounded = value.round();
    let cleaned = if (value - rounded).abs() < SNAP {
        rounded
    } else {
        (value * 1e6).round() / 1e6
    };
    cleaned + 0.0
}

/// [`clean`] on every component.
pub fn clean_vec3(point: DVec3) -> DVec3 {
    DVec3::new(clean(point.x), clean(point.y), clean(point.z))
}

/// A matrix that applies `linear` around `pivot`: the pivot stays where it is.
pub fn around(pivot: DVec3, linear: DMat3) -> DMat4 {
    DMat4::from_translation(pivot) * DMat4::from_mat3(linear) * DMat4::from_translation(-pivot)
}

/// The linear part of an affine matrix.
pub fn linear_part(matrix: &DMat4) -> DMat3 {
    DMat3::from_mat4(*matrix)
}

/// Whether a matrix turns shapes inside out, as a mirror does.
pub fn is_mirror(matrix: &DMat4) -> bool {
    linear_part(matrix).determinant() < 0.0
}

/// Whether a matrix can be applied to brushes: it must not flatten them.
pub fn is_invertible(matrix: &DMat4) -> bool {
    linear_part(matrix).determinant().abs() > 1e-9 && matrix.is_finite()
}

/// The rotation Source builds from an entity's `angles` (pitch, yaw and roll in degrees):
/// roll about X, then pitch about Y, then yaw about Z. Positive pitch looks down.
pub fn angles_matrix(angles: DVec3) -> DMat3 {
    let [pitch, yaw, roll] = angles.to_array().map(f64::to_radians);
    DMat3::from_rotation_z(yaw) * DMat3::from_rotation_y(pitch) * DMat3::from_rotation_x(roll)
}

/// The `angles` of a rotation matrix, as Source's `MatrixAngles` computes them, with yaw
/// from 0 to 360 and pitch and roll from -180 to 180.
pub fn matrix_angles(rotation: DMat3) -> DVec3 {
    let (forward, left, up) = (rotation.x_axis, rotation.y_axis, rotation.z_axis);
    let flat = forward.x.hypot(forward.y);
    let (yaw, pitch, roll) = if flat > 0.001 {
        (forward.y.atan2(forward.x), (-forward.z).atan2(flat), left.z.atan2(up.z))
    } else {
        ((-left.x).atan2(left.y), (-forward.z).atan2(flat), 0.0)
    };
    let degrees = |radians: f64| (radians.to_degrees() * 1000.0).round() / 1000.0 + 0.0;
    let yaw = degrees(yaw).rem_euclid(360.0);
    let yaw = if yaw >= 359.9995 { 0.0 } else { yaw };
    DVec3::new(degrees(pitch), yaw, degrees(roll))
}

/// The angles an entity facing `angles` has after `linear` is applied to it. Its forward
/// and up directions are carried through and squared up again, so mirrors and skews
/// still give a proper rotation. `None` when the transform flattens a direction.
pub fn transform_angles(angles: DVec3, linear: DMat3) -> Option<DVec3> {
    let rotation = angles_matrix(angles);
    let forward = (linear * rotation.x_axis).normalize_or_zero();
    let raised = linear * rotation.z_axis;
    let up = (raised - forward * raised.dot(forward)).normalize_or_zero();
    if forward == DVec3::ZERO || up == DVec3::ZERO {
        return None;
    }
    Some(matrix_angles(DMat3::from_cols(forward, up.cross(forward), up)))
}

impl Side {
    /// Applies an affine transform to the face: its plane points, its texture axes as
    /// `lock` says, and a displacement's start position, normals and offsets.
    ///
    /// A mirroring transform reverses the plane points so the face still points out of
    /// the brush. Displacements are carried along but their rows are not reordered, so a
    /// mirrored displacement keeps its shape only roughly.
    pub fn apply_matrix(&mut self, matrix: &DMat4, lock: TextureLock) {
        let linear = linear_part(matrix);
        if lock != TextureLock::Off {
            let normal_matrix = linear.inverse().transpose();
            let translation = matrix.w_axis.truncate();
            for axis in [&mut self.uaxis, &mut self.vaxis] {
                if axis.scale == 0.0 {
                    continue;
                }
                let carried = normal_matrix * axis.axis;
                let length = carried.length();
                if length < 1e-12 {
                    continue;
                }
                let direction = carried / length;
                match lock {
                    TextureLock::Off => {}
                    TextureLock::Full => {
                        axis.offset -= translation.dot(carried) / axis.scale;
                        axis.scale /= length;
                    }
                    TextureLock::Align { anchor } => {
                        let coordinate = anchor.dot(axis.axis) / axis.scale + axis.offset;
                        let moved = matrix.transform_point3(anchor);
                        axis.offset = coordinate - moved.dot(direction) / axis.scale;
                    }
                }
                axis.axis = clean_vec3(direction);
                axis.offset = clean(axis.offset);
                axis.scale = clean(axis.scale);
            }
        }
        self.transform(&|point| clean_vec3(matrix.transform_point3(point)));
        if linear.determinant() < 0.0 {
            self.plane.swap(0, 2);
        }
        if let Some(dispinfo) = self.extra.iter_mut().find_map(|item| match item {
            Item::Block(block) if block.name.eq_ignore_ascii_case("dispinfo") => Some(block),
            _ => None,
        }) {
            transform_displacement(dispinfo, linear);
        }
    }
}

impl Solid {
    /// Applies an affine transform to every face of the brush. See [`Side::apply_matrix`].
    pub fn apply_matrix(&mut self, matrix: &DMat4, lock: TextureLock) {
        for side in self.sides_mut() {
            side.apply_matrix(matrix, lock);
        }
    }
}

/// Carries a displacement's vectors through the linear part of a transform: each normal
/// turns and its distance stretches by how much the normal did, and offsets turn with it.
fn transform_displacement(dispinfo: &mut Block, linear: DMat3) {
    let mut stretch: Vec<(String, Vec<f64>)> = Vec::new();
    for child in child_blocks(dispinfo, "normals") {
        for (key, value) in rows(child) {
            let mut lengths = Vec::new();
            *value = map_vectors(value, |vector| {
                let carried = linear * vector;
                let length = carried.length();
                lengths.push(length);
                if length > 1e-12 {
                    carried / length
                } else {
                    vector
                }
            });
            stretch.push((key.to_ascii_lowercase(), lengths));
        }
    }
    for child in child_blocks(dispinfo, "distances") {
        for (key, value) in rows(child) {
            let Some(lengths) = stretch.iter().find(|(row, _)| *row == key.to_ascii_lowercase()) else {
                continue;
            };
            if let Some(numbers) = parse_numbers(value) {
                *value = numbers
                    .iter()
                    .enumerate()
                    .map(|(index, distance)| clean(distance * lengths.1.get(index).copied().unwrap_or(1.0)).to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
            }
        }
    }
    for child in child_blocks(dispinfo, "offsets") {
        for (_, value) in rows(child) {
            *value = map_vectors(value, |vector| linear * vector);
        }
    }
    for child in child_blocks(dispinfo, "offset_normals") {
        for (_, value) in rows(child) {
            *value = map_vectors(value, |vector| (linear * vector).try_normalize().unwrap_or(vector));
        }
    }
}

fn child_blocks<'a>(block: &'a mut Block, name: &'a str) -> impl Iterator<Item = &'a mut Block> + 'a {
    block.items.iter_mut().filter_map(move |item| match item {
        Item::Block(child) if child.name.eq_ignore_ascii_case(name) => Some(child),
        _ => None,
    })
}

/// The `rowN` pairs of a displacement block, as key and mutable value.
fn rows(block: &mut Block) -> impl Iterator<Item = (String, &mut String)> {
    block.items.iter_mut().filter_map(|item| match item {
        Item::Pair(pair) => Some((pair.key.clone(), &mut pair.value)),
        Item::Block(_) => None,
    })
}

/// Rewrites a row of `x y z x y z ...` vectors through `map`. Rows that do not parse are
/// left alone.
fn map_vectors(row: &str, mut map: impl FnMut(DVec3) -> DVec3) -> String {
    let Some(numbers) = parse_numbers(row) else {
        return row.to_owned();
    };
    if numbers.len() % 3 != 0 {
        return row.to_owned();
    }
    numbers
        .as_chunks::<3>()
        .0
        .iter()
        .map(|chunk| {
            let mapped = clean_vec3(map(DVec3::new(chunk[0], chunk[1], chunk[2])));
            format!("{} {} {}", mapped.x, mapped.y, mapped.z)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

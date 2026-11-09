//! Overlays and decals: where `info_overlay` and `infodecal` put their picture on brush
//! faces.
//!
//! An overlay is a quad lying in a plane of its own. `BasisOrigin` is a point of that
//! plane, `BasisU` and `BasisV` are the directions its picture runs along, `BasisNormal`
//! points out of the faces it lies on, and `uv0` to `uv3` are the quad's corners, measured
//! from the origin along U and V. The compilers push the quad along the normal onto the
//! faces listed in `sides` and cut it to their shape.
//!
//! The four corners take the texture coordinates (`StartU`, `StartV`), (`StartU`, `EndV`),
//! (`EndU`, `EndV`) and (`EndU`, `StartV`). The game only draws an overlay whose corners
//! run clockwise seen from in front of its faces, against its normal. Hammer gets there
//! with U to the picture's right, V up it (`U x V` is the normal), the first corner at
//! the bottom left, and `StartV` 1 and `EndV` 0 so the picture is not upside down. An
//! overlay wound the other way compiles, and the game leaves it out.

use glam::{DMat3, DMat4, DVec2, DVec3};

use crate::geometry::{clip_polygon, Plane};
use crate::transform::{clean, linear_part, TextureLock};
use crate::{format_vec3, parse_vec3, Entity, TexAxis};

/// The class of overlay entities.
pub const OVERLAY_CLASS: &str = "info_overlay";
/// The class of decal entities.
pub const DECAL_CLASS: &str = "infodecal";
/// How far from a brush face an `infodecal` may be and still land on it. The game looks
/// five units along each axis around the entity.
pub const DECAL_REACH: f64 = 8.0;
/// World units per texel of a new overlay, the scale new brush faces get too.
pub const OVERLAY_TEXEL_SIZE: f64 = 0.25;
/// Pieces smaller than this many square units are dropped as slivers.
const MIN_PIECE_AREA: f64 = 0.01;
/// The keys that hold an overlay's corners.
const CORNER_KEYS: [&str; 4] = ["uv0", "uv1", "uv2", "uv3"];

/// Where an overlay lies: the plane of its quad and the quad's four corners.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overlay {
    /// A point of the overlay's plane, which the corners are measured from.
    pub origin: DVec3,
    /// The direction the picture's width runs along.
    pub u: DVec3,
    /// The direction the picture's height runs along.
    pub v: DVec3,
    /// The direction out of the faces the overlay lies on.
    pub normal: DVec3,
    /// The corners along `u` and `v` from the origin, in the order `uv0` to `uv3`.
    pub corners: [DVec2; 4],
}

/// The texture coordinate keys of an overlay Hammer makes, which show the picture upright
/// on the axes of [`Overlay::on_surface`].
pub const UPRIGHT_TEXTURE_KEYS: [(&str, &str); 4] = [("StartU", "0"), ("EndU", "1"), ("StartV", "1"), ("EndV", "0")];
/// The texture coordinates [`UPRIGHT_TEXTURE_KEYS`] give the four corners.
pub const UPRIGHT_TEXTURE: [DVec2; 4] = [DVec2::Y, DVec2::ZERO, DVec2::X, DVec2::ONE];

/// One convex piece of an overlay or decal on a brush face. A point's texture coordinate
/// is `point · axis + offset` on each axis, from 0 to 1 across the picture whatever the
/// texture's size.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    /// The piece's corners, on the face.
    pub vertices: Vec<DVec3>,
    /// The face's outward normal.
    pub normal: DVec3,
    pub uaxis: TexAxis,
    pub vaxis: TexAxis,
}

/// The directions a picture's width and height run along on a surface facing `normal`,
/// as the game lays decals out: upright on walls, and with its top to the north on
/// floors and ceilings.
pub fn surface_axes(normal: DVec3) -> (DVec3, DVec3) {
    let normal = normal.normalize_or(DVec3::Z);
    let (u, v) = if normal.z.abs() > std::f64::consts::FRAC_1_SQRT_2 {
        let v = DVec3::X.cross(normal);
        (normal.cross(v), v)
    } else {
        let u = normal.cross(DVec3::NEG_Z);
        (u, u.cross(normal))
    };
    (tidy(u.normalize()), tidy(v.normalize()))
}

/// The texture coordinates at an overlay's four corners, from its `StartU`, `EndU`,
/// `StartV` and `EndV` keys.
pub fn texture_corners(entity: &Entity) -> [DVec2; 4] {
    let number = |key: &str, default: f64| {
        entity
            .get(key)
            .and_then(|value| value.trim().parse::<f64>().ok())
            .unwrap_or(default)
    };
    let (start, end) = (
        DVec2::new(number("StartU", 0.0), number("StartV", 0.0)),
        DVec2::new(number("EndU", 1.0), number("EndV", 1.0)),
    );
    [start, DVec2::new(start.x, end.y), end, DVec2::new(end.x, start.y)]
}

impl Overlay {
    /// Reads an overlay's placement from its entity. `None` when a basis key or a corner
    /// is missing or malformed, as for an overlay that was never placed on a face.
    pub fn read(entity: &Entity) -> Option<Self> {
        let vector = |key: &str| entity.get(key).and_then(parse_vec3);
        let mut corners = [DVec2::ZERO; 4];
        for (corner, key) in corners.iter_mut().zip(CORNER_KEYS) {
            *corner = vector(key)?.truncate();
        }
        Some(Self {
            origin: vector("BasisOrigin").or_else(|| entity.origin())?,
            u: vector("BasisU")?,
            v: vector("BasisV")?,
            normal: vector("BasisNormal")?,
            corners,
        })
    }

    /// The placement of an `info_overlay` entity. `None` for other classes and for an
    /// overlay that was never placed.
    pub fn of(entity: &Entity) -> Option<Self> {
        entity
            .classname()
            .eq_ignore_ascii_case(OVERLAY_CLASS)
            .then(|| Self::read(entity))
            .flatten()
    }

    /// An overlay or decal `size` units wide and high, centred on `point` of a surface
    /// facing `normal`, laid out as Hammer does: U to the picture's right and V up it, so
    /// it shows upright with [`UPRIGHT_TEXTURE_KEYS`] and the game draws it.
    pub fn on_surface(point: DVec3, normal: DVec3, size: DVec2) -> Self {
        let (u, down) = surface_axes(normal);
        let half = size * 0.5;
        Self {
            origin: point,
            u,
            v: tidy(-down),
            normal: tidy(normal.normalize_or(DVec3::Z)),
            corners: [
                DVec2::new(-half.x, -half.y),
                DVec2::new(-half.x, half.y),
                DVec2::new(half.x, half.y),
                DVec2::new(half.x, -half.y),
            ],
        }
    }

    /// The keys that store the placement, with `origin` following the basis origin as
    /// Hammer keeps it.
    pub fn keys(&self) -> Vec<(&'static str, String)> {
        let mut keys = vec![
            ("BasisOrigin", format_vec3(self.origin)),
            ("BasisU", format_vec3(self.u)),
            ("BasisV", format_vec3(self.v)),
            ("BasisNormal", format_vec3(self.normal)),
        ];
        for (key, corner) in CORNER_KEYS.into_iter().zip(self.corners) {
            keys.push((key, format_vec3(corner.extend(0.0))));
        }
        keys.push(("origin", format_vec3(self.origin)));
        keys
    }

    /// Writes the placement into an entity.
    pub fn write(&self, entity: &mut Entity) {
        for (key, value) in self.keys() {
            entity.set(key, value);
        }
    }

    /// The quad's corners in the world.
    pub fn world_corners(&self) -> [DVec3; 4] {
        self.corners
            .map(|corner| self.origin + self.u * corner.x + self.v * corner.y)
    }

    /// Whether the game draws the overlay: its corners run clockwise seen from in front
    /// of its faces. An overlay wound the other way is compiled into the map and then
    /// left out.
    pub fn shows_in_game(&self) -> bool {
        let [a, b, c, _] = self.world_corners();
        (b - a).cross(c - b).dot(self.normal) < 0.0
    }

    /// The overlay carried through an affine transform: moved, turned, mirrored or
    /// stretched with whatever it is selected with. The axes stay at right angles, so a
    /// skew bends the corners instead. `None` when the transform flattens the overlay.
    ///
    /// A mirror would turn the corners the other way round, which the game does not
    /// draw. So after one, U is turned back and the corners are numbered in reverse; the
    /// second value is then `true`, and the caller has to swap `StartU` and `EndU` for
    /// the picture to stay on the corners it was on, mirrored with everything else.
    pub fn transformed(&self, matrix: &DMat4) -> Option<(Self, bool)> {
        let linear = linear_part(matrix);
        let (mut u, v, normal) = self.carried_axes(&linear)?;
        let mirrored = linear.determinant() < 0.0;
        if mirrored {
            u = -u;
        }
        let origin = matrix.transform_point3(self.origin);
        let mut corners = self.world_corners().map(|corner| {
            let moved = matrix.transform_point3(corner) - origin;
            DVec2::new(tidy_number(moved.dot(u)), tidy_number(moved.dot(v)))
        });
        if mirrored {
            corners.reverse();
        }
        let moved = Self {
            origin: tidy(origin),
            u: tidy(u),
            v: tidy(v),
            normal: tidy(normal),
            corners,
        };
        Some((moved, mirrored))
    }

    /// The overlay carried along by a transform without changing shape: it goes where
    /// the transform takes its origin and turns to lie on the transformed plane, but a
    /// stretch or skew leaves its width, height and corners as they are. This is how an
    /// overlay follows a brush whose textures keep their scale. The second value is as
    /// for [`Overlay::transformed`].
    pub fn carried(&self, matrix: &DMat4) -> Option<(Self, bool)> {
        let (u, v, normal) = self.carried_axes(&linear_part(matrix))?;
        let from = DMat3::from_cols(self.u, self.v, self.normal);
        if from.determinant().abs() < 1e-9 {
            return None;
        }
        let turn = DMat3::from_cols(u, v, normal) * from.inverse();
        let rigid = DMat4::from_translation(matrix.transform_point3(self.origin))
            * DMat4::from_mat3(turn)
            * DMat4::from_translation(-self.origin);
        self.transformed(&rigid)
    }

    /// Where a transform's linear part takes the overlay's axes and normal, squared up
    /// again: the normal stays at right angles to the transformed plane, U keeps its
    /// direction in that plane, and V is turned to stand at right angles to both. `None`
    /// when the transform flattens the overlay.
    fn carried_axes(&self, linear: &DMat3) -> Option<(DVec3, DVec3, DVec3)> {
        if linear.determinant().abs() < 1e-12 {
            return None;
        }
        let normal = (linear.inverse().transpose() * self.normal).try_normalize()?;
        let across = *linear * self.u;
        let u = (across - normal * across.dot(normal)).try_normalize()?;
        let up = *linear * self.v;
        let v = (up - normal * up.dot(normal) - u * up.dot(u)).try_normalize()?;
        Some((u, v, normal))
    }

    /// Adds to `out` the pieces of the overlay that land on a face with the corners
    /// `polygon` and the outward normal `normal`. `texture` holds the texture coordinates
    /// at the overlay's corners. A face seen edge-on from the overlay gets nothing, and
    /// neither does any face when the game would not draw the overlay.
    pub fn pieces_on(&self, texture: &[DVec2; 4], polygon: &[DVec3], normal: DVec3, out: &mut Vec<Piece>) {
        let frame = DMat3::from_cols(self.u, self.v, self.normal);
        let unusable = polygon.len() < 3 || frame.determinant().abs() < 1e-9 || normal.dot(self.normal).abs() < 1e-3;
        if unusable || !self.shows_in_game() {
            return;
        }
        let inverse = frame.inverse();
        let (along_u, along_v) = (inverse.row(0), inverse.row(1));
        let world = self.world_corners();
        for triangle in [[0, 1, 2], [0, 2, 3]] {
            let flat = triangle.map(|index| self.corners[index]);
            if (flat[1] - flat[0]).perp_dot(flat[2] - flat[0]).abs() < 1e-9 {
                continue;
            }
            let mut piece = polygon.to_vec();
            for edge in 0..3 {
                let [a, b, c] = [0, 1, 2].map(|step| world[triangle[(edge + step) % 3]]);
                let Some(mut outward) = (b - a).cross(self.normal).try_normalize() else {
                    piece.clear();
                    break;
                };
                if outward.dot(c - a) > 0.0 {
                    outward = -outward;
                }
                piece = clip_polygon(
                    &piece,
                    &Plane {
                        normal: outward,
                        dist: outward.dot(a),
                    },
                );
                if piece.len() < 3 {
                    break;
                }
            }
            if piece.len() < 3 || polygon_area(&piece) < MIN_PIECE_AREA {
                continue;
            }
            let solve = DMat3::from_cols(
                DVec3::new(flat[0].x, flat[1].x, flat[2].x),
                DVec3::new(flat[0].y, flat[1].y, flat[2].y),
                DVec3::ONE,
            )
            .inverse();
            let axis = |pick: fn(DVec2) -> f64| {
                let [a, b, c] = triangle.map(|index| pick(texture[index]));
                let weights = solve * DVec3::new(a, b, c);
                let axis = along_u * weights.x + along_v * weights.y;
                TexAxis {
                    axis,
                    offset: weights.z - axis.dot(self.origin),
                    scale: 1.0,
                }
            };
            out.push(Piece {
                vertices: piece,
                normal,
                uaxis: axis(|point| point.x),
                vaxis: axis(|point| point.y),
            });
        }
    }
}

/// The key changes that carry an entity's overlay through a transform, or nothing when
/// the entity is not a placed overlay. A mirror also swaps `StartU` and `EndU`, as
/// [`Overlay::transformed`] asks.
pub fn transform_keys(entity: &Entity, matrix: &DMat4) -> Vec<(&'static str, String)> {
    moved_keys(
        entity,
        Overlay::of(entity).and_then(|overlay| overlay.transformed(matrix)),
    )
}

/// The key changes that take an overlay along with the brush faces it lies on when those
/// are transformed, the way `lock` takes their textures along: not at all without
/// texture lock, stretching with the faces under texture scale lock, and otherwise
/// moving and turning with them at its own size.
pub fn follow_keys(entity: &Entity, matrix: &DMat4, lock: TextureLock) -> Vec<(&'static str, String)> {
    let overlay = Overlay::of(entity);
    let moved = match lock {
        TextureLock::Off => None,
        TextureLock::Full => overlay.and_then(|overlay| overlay.transformed(matrix)),
        TextureLock::Align { .. } => overlay.and_then(|overlay| overlay.carried(matrix)),
    };
    moved_keys(entity, moved)
}

/// The keys of an overlay after a transform, with `StartU` and `EndU` swapped when it
/// was mirrored. Nothing when there is no moved overlay.
fn moved_keys(entity: &Entity, moved: Option<(Overlay, bool)>) -> Vec<(&'static str, String)> {
    let Some((moved, mirrored)) = moved else {
        return Vec::new();
    };
    let mut keys = moved.keys();
    if mirrored {
        let value = |key: &str, default: &str| entity.get(key).unwrap_or(default).to_owned();
        keys.push(("StartU", value("EndU", "1")));
        keys.push(("EndU", value("StartU", "0")));
    }
    keys
}

/// Carries an entity's overlay through a transform in place. Entities that are not
/// placed overlays are left alone.
pub fn transform_entity(entity: &mut Entity, matrix: &DMat4) {
    for (key, value) in transform_keys(entity, matrix) {
        entity.set(key, value);
    }
}

/// How far a point is from a convex polygon, and the polygon's point nearest to it.
pub fn nearest_on_polygon(point: DVec3, polygon: &[DVec3], normal: DVec3) -> Option<(f64, DVec3)> {
    let first = *polygon.first()?;
    let on_plane = point - normal * normal.dot(point - first);
    let count = polygon.len();
    let turn = |index: usize, to: DVec3| {
        let (a, b) = (polygon[index], polygon[(index + 1) % count]);
        (b - a).cross(to - a).dot(normal)
    };
    if count >= 3 {
        let winding = turn(0, polygon[2]).signum();
        if (0..count).all(|index| turn(index, on_plane) * winding >= -1e-9) {
            return Some((point.distance(on_plane), on_plane));
        }
    }
    (0..count)
        .map(|index| {
            let (a, b) = (polygon[index], polygon[(index + 1) % count]);
            let along = b - a;
            let length = along.length_squared();
            let t = if length > 0.0 {
                ((point - a).dot(along) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let nearest = a + along * t;
            (point.distance(nearest), nearest)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// The area of a flat polygon.
fn polygon_area(polygon: &[DVec3]) -> f64 {
    let first = polygon[0];
    polygon
        .windows(2)
        .skip(1)
        .map(|pair| (pair[0] - first).cross(pair[1] - first))
        .sum::<DVec3>()
        .length()
        * 0.5
}

/// Rounds a number to a millionth, and onto a whole number when it is that close to one,
/// so axes and corners are written without rounding noise.
fn tidy_number(value: f64) -> f64 {
    clean((value * 1e6).round() / 1e6) + 0.0
}

fn tidy(vector: DVec3) -> DVec3 {
    DVec3::new(tidy_number(vector.x), tidy_number(vector.y), tidy_number(vector.z))
}

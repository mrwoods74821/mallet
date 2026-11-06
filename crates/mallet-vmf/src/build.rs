//! Builders for new brushes, matching what Hammer's tools create.

use glam::DVec3;
use mallet_kv::{Block, Pair};

use crate::{IdAllocator, Side, Solid, SolidItem, TexAxis};

/// Texture scale Hammer applies to new faces.
const DEFAULT_TEXTURE_SCALE: f64 = 0.25;
/// Luxels per world unit Hammer applies to new faces.
const DEFAULT_LIGHTMAP_SCALE: u32 = 16;

/// Builds an axis-aligned box brush between two corners, with every face using `material`
/// and world-aligned texture axes like Hammer's block tool.
///
/// Returns `None` when the box has no volume.
pub fn box_solid(min: DVec3, max: DVec3, material: &str, ids: &mut IdAllocator) -> Option<Solid> {
    let (min, max) = (min.min(max), min.max(max));
    if (max - min).min_element() <= 0.0 {
        return None;
    }
    let [x0, y0, z0] = min.to_array();
    let [x1, y1, z1] = max.to_array();
    let p = DVec3::new;
    let faces = [
        ([p(x0, y1, z1), p(x1, y1, z1), p(x1, y0, z1)], DVec3::X, DVec3::NEG_Y),
        ([p(x0, y0, z0), p(x1, y0, z0), p(x1, y1, z0)], DVec3::X, DVec3::NEG_Y),
        ([p(x0, y1, z1), p(x0, y0, z1), p(x0, y0, z0)], DVec3::Y, DVec3::NEG_Z),
        ([p(x1, y1, z0), p(x1, y0, z0), p(x1, y0, z1)], DVec3::Y, DVec3::NEG_Z),
        ([p(x1, y1, z1), p(x0, y1, z1), p(x0, y1, z0)], DVec3::X, DVec3::NEG_Z),
        ([p(x1, y0, z0), p(x0, y0, z0), p(x0, y0, z1)], DVec3::X, DVec3::NEG_Z),
    ];

    let id = ids.object();
    let mut items = vec![SolidItem::Key(Pair::new("id", id.to_string()))];
    for (plane, u, v) in faces {
        let axis = |axis| TexAxis {
            axis,
            offset: 0.0,
            scale: DEFAULT_TEXTURE_SCALE,
        };
        items.push(SolidItem::Side(Side {
            id: ids.side(),
            plane,
            material: material.to_owned(),
            uaxis: axis(u),
            vaxis: axis(v),
            rotation: Some(0.0),
            lightmap_scale: Some(DEFAULT_LIGHTMAP_SCALE),
            smoothing_groups: Some(0),
            extra: Vec::new(),
        }));
    }
    let mut editor = Block::new("editor");
    editor.push_pair("color", editor_color(id));
    editor.push_pair("visgroupshown", "1");
    editor.push_pair("visgroupautoshown", "1");
    items.push(SolidItem::Other(editor));
    Some(Solid { items })
}

/// A pseudo-random green-blue colour per brush, like the colours Hammer picks.
fn editor_color(id: u32) -> String {
    let green = 100 + id.wrapping_mul(37) % 156;
    let blue = 100 + id.wrapping_mul(91) % 156;
    format!("0 {green} {blue}")
}

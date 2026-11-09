//! Entity keys that list brush faces by id, kept right when a copy gets fresh ids.
//!
//! An overlay names the faces it lies on in `sides`. A copied brush gets new face ids,
//! so an overlay copied along with it has to name the faces of the copy instead.

use std::collections::HashMap;

use crate::{Entity, IdAllocator, Solid};

/// The keys that list brush faces by id: `sides` on `info_overlay`, `env_cubemap` and
/// `info_no_dynamic_shadow`, and `sides2` as well on `info_overlay_transition`.
pub const SIDE_LIST_KEYS: [&str; 2] = ["sides", "sides2"];

/// What becomes of a listed face that was not copied along with the entity listing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherFaces {
    /// It stays listed, for a copy made in the map the face is in.
    Keep,
    /// It is taken off the list, for a copy that may land in another map, where the id
    /// would name an unrelated face.
    Drop,
}

/// The new id of every brush face copied so far, by the id it was copied from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CopiedSides {
    new_ids: HashMap<u32, u32>,
}

impl CopiedSides {
    /// Gives a copied brush and its faces fresh ids, as [`Solid::renumber`] does, and
    /// remembers which face became which.
    pub fn renumber_solid(&mut self, solid: &mut Solid, ids: &mut IdAllocator) {
        let before: Vec<u32> = solid.sides().map(|side| side.id).collect();
        solid.renumber(ids);
        self.new_ids
            .extend(before.into_iter().zip(solid.sides().map(|side| side.id)));
    }

    /// Gives a copied entity, its brushes and their faces fresh ids, as
    /// [`Entity::renumber`] does, and remembers which face became which.
    pub fn renumber_entity(&mut self, entity: &mut Entity, ids: &mut IdAllocator) {
        let before: Vec<u32> = entity_sides(entity).collect();
        entity.renumber(ids);
        self.new_ids.extend(before.into_iter().zip(entity_sides(entity)));
    }

    /// The id a face has in the copy, if the face was copied.
    pub fn new_id(&self, old: u32) -> Option<u32> {
        self.new_ids.get(&old).copied()
    }

    /// Rewrites a space-separated list of face ids, such as an overlay's `sides`, so it
    /// names the copied faces. Faces that were not copied stay or go as `others` says, and
    /// no face is listed twice.
    ///
    /// Returns `None` when the list stays as it is, which includes text that is not a
    /// list of ids.
    pub fn remap_list(&self, list: &str, others: OtherFaces) -> Option<String> {
        let mut changed = false;
        let mut out: Vec<u32> = Vec::new();
        for word in list.split_whitespace() {
            let old = word.parse::<u32>().ok()?;
            let id = match (self.new_id(old), others) {
                (Some(new), _) => new,
                (None, OtherFaces::Keep) => old,
                (None, OtherFaces::Drop) => {
                    changed = true;
                    continue;
                }
            };
            changed |= id != old;
            if out.contains(&id) {
                changed = true;
            } else {
                out.push(id);
            }
        }
        changed.then(|| out.iter().map(u32::to_string).collect::<Vec<_>>().join(" "))
    }

    /// Points every face list of a copied entity at the copied faces. Entities that list
    /// no faces are left untouched.
    pub fn remap_entity(&self, entity: &mut Entity, others: OtherFaces) {
        for key in SIDE_LIST_KEYS {
            if let Some(list) = entity.get(key).and_then(|list| self.remap_list(list, others)) {
                entity.set(key, list);
            }
        }
    }
}

/// The ids of the faces of an entity's brushes, in file order.
fn entity_sides(entity: &Entity) -> impl Iterator<Item = u32> + '_ {
    entity.solids().flat_map(Solid::sides).map(|side| side.id)
}

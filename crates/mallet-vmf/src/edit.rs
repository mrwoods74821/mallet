//! Reversible edits and an undo history.
//!
//! Every change to a map, whether it comes from the editor UI or from an automation client
//! such as an MCP server, is expressed as an [`Edit`]. Applying an edit returns its inverse,
//! which is what makes undo and redo work the same way for every caller.

use std::error::Error;
use std::fmt;

use glam::DVec3;

use crate::{Entity, EntityItem, Solid, TopLevel, Vmf};

/// One reversible change to a map.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Sets a key on the entity with the given id, adding it if missing.
    SetKey {
        /// Target entity id; the world has id 1 in maps Hammer saves.
        entity: u32,
        /// Key name.
        key: String,
        /// New value.
        value: String,
    },
    /// Removes a key from the entity with the given id.
    ///
    /// Undoing a removal re-adds the key after the entity's last key, so its position in
    /// the file may change.
    RemoveKey {
        /// Target entity id.
        entity: u32,
        /// Key name.
        key: String,
    },
    /// Adds a brush to the world or to a brush entity.
    AddSolid {
        /// Owning entity id; use the world's id for world brushes.
        entity: u32,
        /// Position among the entity's items, or `None` to add it after its last brush.
        index: Option<usize>,
        /// The brush, which must already have unique ids.
        solid: Solid,
    },
    /// Removes the brush with the given id from whichever entity owns it.
    RemoveSolid {
        /// Brush id.
        solid: u32,
    },
    /// Adds an entity.
    AddEntity {
        /// Position among the map's top-level items, or `None` to add it after the last entity.
        index: Option<usize>,
        /// The entity, which must already have unique ids.
        entity: Entity,
    },
    /// Removes the entity with the given id. The world cannot be removed.
    RemoveEntity {
        /// Entity id.
        entity: u32,
    },
    /// Moves brushes with texture lock. Entity origins are moved with [`Edit::SetKey`].
    Translate {
        /// Brush ids.
        solids: Vec<u32>,
        /// Distance to move.
        offset: DVec3,
    },
    /// Several edits applied in order and undone in reverse as a single step.
    Batch(Vec<Edit>),
}

/// Why an edit could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// No world or entity has this id.
    NoSuchEntity(u32),
    /// No brush has this id.
    NoSuchSolid(u32),
    /// The entity has no such key to remove.
    NoSuchKey {
        /// Target entity id.
        entity: u32,
        /// Missing key.
        key: String,
    },
    /// The world entity is part of every map and cannot be removed.
    CannotRemoveWorld,
    /// A brush or entity being added has no numeric id, so it could not be undone.
    MissingId,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchEntity(id) => write!(f, "no entity has id {id}"),
            Self::NoSuchSolid(id) => write!(f, "no brush has id {id}"),
            Self::NoSuchKey { entity, key } => write!(f, "entity {entity} has no key \"{key}\""),
            Self::CannotRemoveWorld => write!(f, "the world cannot be removed"),
            Self::MissingId => write!(f, "the added brush or entity has no id"),
        }
    }
}

impl Error for EditError {}

impl Edit {
    /// Applies the edit and returns the edit that undoes it.
    ///
    /// An edit that fails changes nothing: a batch that fails part-way is rolled back
    /// before the error is returned.
    pub fn apply(&self, vmf: &mut Vmf) -> Result<Edit, EditError> {
        match self {
            Self::SetKey { entity, key, value } => {
                let target = entity_mut(vmf, *entity)?;
                let inverse = match target.get(key) {
                    Some(old) => Self::SetKey {
                        entity: *entity,
                        key: key.clone(),
                        value: old.to_owned(),
                    },
                    None => Self::RemoveKey {
                        entity: *entity,
                        key: key.clone(),
                    },
                };
                target.set(key, value.clone());
                Ok(inverse)
            }
            Self::RemoveKey { entity, key } => {
                let target = entity_mut(vmf, *entity)?;
                let old = target.remove(key).ok_or_else(|| EditError::NoSuchKey {
                    entity: *entity,
                    key: key.clone(),
                })?;
                Ok(Self::SetKey {
                    entity: *entity,
                    key: key.clone(),
                    value: old,
                })
            }
            Self::AddSolid { entity, index, solid } => {
                let id = solid.id().ok_or(EditError::MissingId)?;
                let target = entity_mut(vmf, *entity)?;
                let at = index
                    .unwrap_or_else(|| default_solid_index(target))
                    .min(target.items.len());
                target.items.insert(at, EntityItem::Solid(solid.clone()));
                Ok(Self::RemoveSolid { solid: id })
            }
            Self::RemoveSolid { solid } => {
                let (entity_index, item_index) = find_solid(vmf, *solid).ok_or(EditError::NoSuchSolid(*solid))?;
                let owner = top_level_entity_mut(vmf, entity_index);
                let entity = owner.id().ok_or(EditError::MissingId)?;
                let EntityItem::Solid(removed) = owner.items.remove(item_index) else {
                    unreachable!("find_solid only returns solid positions");
                };
                Ok(Self::AddSolid {
                    entity,
                    index: Some(item_index),
                    solid: removed,
                })
            }
            Self::AddEntity { index, entity } => {
                let id = entity.id().ok_or(EditError::MissingId)?;
                let at = index
                    .unwrap_or_else(|| default_entity_index(vmf))
                    .min(vmf.items.len());
                vmf.items.insert(at, TopLevel::Entity(entity.clone()));
                Ok(Self::RemoveEntity { entity: id })
            }
            Self::RemoveEntity { entity } => {
                let index = vmf
                    .items
                    .iter()
                    .position(|item| matches!(item, TopLevel::Entity(e) if e.id() == Some(*entity)));
                let Some(index) = index else {
                    let is_world = vmf.world().is_some_and(|world| world.id() == Some(*entity));
                    return Err(if is_world {
                        EditError::CannotRemoveWorld
                    } else {
                        EditError::NoSuchEntity(*entity)
                    });
                };
                let TopLevel::Entity(removed) = vmf.items.remove(index) else {
                    unreachable!("position matched an entity");
                };
                Ok(Self::AddEntity {
                    index: Some(index),
                    entity: removed,
                })
            }
            Self::Translate { solids, offset } => {
                if let Some(missing) = solids.iter().find(|id| find_solid(vmf, **id).is_none()) {
                    return Err(EditError::NoSuchSolid(*missing));
                }
                for id in solids {
                    if let Some(solid) = solid_mut(vmf, *id) {
                        solid.translate(*offset);
                    }
                }
                Ok(Self::Translate {
                    solids: solids.clone(),
                    offset: -*offset,
                })
            }
            Self::Batch(edits) => {
                let mut inverses = Vec::with_capacity(edits.len());
                for edit in edits {
                    match edit.apply(vmf) {
                        Ok(inverse) => inverses.push(inverse),
                        Err(error) => {
                            for inverse in inverses.iter().rev() {
                                let _ = inverse.apply(vmf);
                            }
                            return Err(error);
                        }
                    }
                }
                inverses.reverse();
                Ok(Self::Batch(inverses))
            }
        }
    }
}

fn entity_mut(vmf: &mut Vmf, id: u32) -> Result<&mut Entity, EditError> {
    vmf.items
        .iter_mut()
        .find_map(|item| match item {
            TopLevel::World(entity) | TopLevel::Entity(entity) if entity.id() == Some(id) => Some(entity),
            _ => None,
        })
        .ok_or(EditError::NoSuchEntity(id))
}

fn top_level_entity_mut(vmf: &mut Vmf, index: usize) -> &mut Entity {
    match &mut vmf.items[index] {
        TopLevel::World(entity) | TopLevel::Entity(entity) => entity,
        _ => unreachable!("find_solid only returns entity positions"),
    }
}

/// Finds a brush by id: the index of its owner in the map's items and its index inside
/// the owner.
fn find_solid(vmf: &Vmf, id: u32) -> Option<(usize, usize)> {
    vmf.items.iter().enumerate().find_map(|(entity_index, item)| {
        let (TopLevel::World(entity) | TopLevel::Entity(entity)) = item else {
            return None;
        };
        entity
            .items
            .iter()
            .position(|item| matches!(item, EntityItem::Solid(solid) if solid.id() == Some(id)))
            .map(|item_index| (entity_index, item_index))
    })
}

fn solid_mut(vmf: &mut Vmf, id: u32) -> Option<&mut Solid> {
    let (entity_index, item_index) = find_solid(vmf, id)?;
    match &mut top_level_entity_mut(vmf, entity_index).items[item_index] {
        EntityItem::Solid(solid) => Some(solid),
        _ => None,
    }
}

/// Just after the entity's last brush, or after its keys when it has none.
fn default_solid_index(entity: &Entity) -> usize {
    entity
        .items
        .iter()
        .rposition(|item| matches!(item, EntityItem::Solid(_)))
        .or_else(|| entity.items.iter().rposition(|item| matches!(item, EntityItem::Key(_))))
        .map_or(0, |index| index + 1)
}

/// Just after the map's last entity or world block.
fn default_entity_index(vmf: &Vmf) -> usize {
    vmf.items
        .iter()
        .rposition(|item| matches!(item, TopLevel::World(_) | TopLevel::Entity(_)))
        .map_or(vmf.items.len(), |index| index + 1)
}

/// Undo and redo stacks of applied edits.
#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
}

impl History {
    /// Applies an edit, records it for undo and clears the redo stack.
    pub fn apply(&mut self, vmf: &mut Vmf, edit: &Edit) -> Result<(), EditError> {
        let inverse = edit.apply(vmf)?;
        self.undo.push(inverse);
        self.redo.clear();
        Ok(())
    }

    /// Undoes the latest edit. Returns `false` when there is nothing to undo.
    pub fn undo(&mut self, vmf: &mut Vmf) -> Result<bool, EditError> {
        let Some(edit) = self.undo.pop() else {
            return Ok(false);
        };
        let inverse = edit.apply(vmf)?;
        self.redo.push(inverse);
        Ok(true)
    }

    /// Redoes the latest undone edit. Returns `false` when there is nothing to redo.
    pub fn redo(&mut self, vmf: &mut Vmf) -> Result<bool, EditError> {
        let Some(edit) = self.redo.pop() else {
            return Ok(false);
        };
        let inverse = edit.apply(vmf)?;
        self.undo.push(inverse);
        Ok(true)
    }

    /// Whether an undo step is available.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether a redo step is available.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

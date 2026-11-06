//! Reversible edits and an undo history.
//!
//! Every change to a map, whether it comes from the editor UI or from an automation client
//! such as an MCP server, is expressed as an [`Edit`]. Applying an edit returns its inverse,
//! which is what makes undo and redo work the same way for every caller.

use std::error::Error;
use std::fmt;

use crate::{TopLevel, Vmf};

/// One reversible change to a map.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Several edits applied in order and undone in reverse as a single step.
    Batch(Vec<Edit>),
}

/// Why an edit could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// No world or entity has this id.
    NoSuchEntity(u32),
    /// The entity has no such key to remove.
    NoSuchKey {
        /// Target entity id.
        entity: u32,
        /// Missing key.
        key: String,
    },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchEntity(id) => write!(f, "no entity has id {id}"),
            Self::NoSuchKey { entity, key } => write!(f, "entity {entity} has no key \"{key}\""),
        }
    }
}

impl Error for EditError {}

impl Edit {
    /// Applies the edit and returns the edit that undoes it.
    ///
    /// A batch that fails part-way is rolled back before the error is returned, so the map
    /// is never left half-edited.
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

fn entity_mut(vmf: &mut Vmf, id: u32) -> Result<&mut crate::Entity, EditError> {
    vmf.items
        .iter_mut()
        .find_map(|item| match item {
            TopLevel::World(entity) | TopLevel::Entity(entity) if entity.id() == Some(id) => Some(entity),
            _ => None,
        })
        .ok_or(EditError::NoSuchEntity(id))
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

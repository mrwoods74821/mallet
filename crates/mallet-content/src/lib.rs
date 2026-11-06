//! Reads Source engine game content the way the engine and Hammer find it.
//!
//! [`GameFs`] mounts the search paths listed in a game's `gameinfo.txt`: loose folders and
//! VPK archives, in priority order. [`Materials`] resolves material names to the texture
//! Hammer shows for them and decodes it from VTF, [`load_model`] reads prop models and [`GameData`] reads
//! the entity classes of FGD files.

mod error;
pub mod fgd;
pub mod filesystem;
pub mod material;
pub mod model;
pub mod vpk;
pub mod vtf;

pub use error::ContentError;
pub use fgd::{default_fgd, Choice, ClassKind, EntityClass, GameData, Port, Property};
pub use filesystem::GameFs;
pub use material::{parallel_map, AlphaMode, Material, MaterialLoader, Materials, Texture};
pub use model::{load_model, Model, ModelMesh, ModelVertex};

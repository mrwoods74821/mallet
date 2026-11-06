//! Reads Source engine game content the way the engine and Hammer find it.
//!
//! [`GameFs`] mounts the search paths listed in a game's `gameinfo.txt`: loose folders and
//! VPK archives, in priority order. [`Materials`] resolves material names to the texture
//! Hammer shows for them and decodes it from VTF.

mod error;
pub mod filesystem;
pub mod material;
pub mod vpk;
pub mod vtf;

pub use error::ContentError;
pub use filesystem::GameFs;
pub use material::{parallel_map, Materials, Texture};

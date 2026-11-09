//! Typed, lossless model of Hammer's VMF map format, plus brush geometry helpers.
//!
//! [`Vmf`] keeps every block it does not understand verbatim, so loading and saving a
//! map changes nothing except number formatting that parses back to the same value.

pub mod build;
pub mod edit;
pub mod geometry;
pub mod hull;
pub mod light;
pub mod lightmap;
mod model;
pub mod texture;
pub mod transform;
mod view;

pub use mallet_kv as kv;
pub use model::*;
pub use view::SavedCamera;

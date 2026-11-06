//! Typed, lossless model of Hammer's VMF map format, plus brush geometry helpers.
//!
//! [`Vmf`] keeps every block it does not understand verbatim, so loading and saving a
//! map changes nothing except number formatting that parses back to the same value.

pub mod geometry;
mod model;

pub use mallet_kv as kv;
pub use model::*;

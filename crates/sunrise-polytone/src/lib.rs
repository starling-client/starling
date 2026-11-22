//! Polytone-format resource pack files (`assets/<namespace>/polytone/...`), the format of the
//! Polytone mod's packs (#11486).
//!
//! Clean room: written from the public wiki (<https://github.com/MehVahdJukaar/polytone/wiki>) and
//! from observing packs, never from the mod's GPL source. This crate only reads the files: no GPU,
//! no gameplay. The game applies what it reads; nothing changes without a Polytone pack.
//!
//! Read so far: [`Targets`] (what a modifier aims at), and the [`BiomeModifier`] and
//! [`DimensionModifier`] environment attribute layers, the scripting expressions
//! ([`expr::Program`]) their values and other files use, and a GPU particle type's own shader
//! with the block of its knobs ([`gpu_particle`]).
mod attributes;
pub mod colors;
pub mod creative_tabs;
pub mod expr;
pub mod globals;
pub mod gpu_particle;
mod modifiers;
pub mod pack;
pub mod particles;
pub mod shader_modifiers;
pub mod sprites;
mod target;

pub use attributes::{Argument, Attributes, Layer, Modifier, normalize_attribute};
pub use modifiers::{BiomeModifier, DimensionModifier};
pub use target::{Target, Targets, implicit_id};

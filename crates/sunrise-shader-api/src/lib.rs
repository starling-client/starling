//! Sunrise's own shader packs (docs/api/shaders.md): WGSL written against
//! the renderer's stages, described by `sunrise-shaders.toml` and shipped as
//! a folder or a `.sshader`. This crate reads and checks packs; the game, the
//! launcher and the registry all use it, so a pack is judged the same way
//! everywhere.
pub mod check;
pub mod guard;
pub mod inputs;
pub mod manifest;
pub mod package;
pub mod reference;
pub mod stages;
pub mod wgsl;

pub use check::{Report, check};
pub use inputs::Frame;
pub use manifest::{
    API, Api, Blend, Coverage, LightColor, MANIFEST, Manifest, Material, OptionKind, OptionPage,
    PackOption, PackTexture, PageItem, Primaries, RayTracing, SCHEMA, SpriteTag, Stage,
};
pub use package::{EXTENSION, Pack};
pub use stages::{StageKind, compose};
pub use wgsl::{Composed, Problem};

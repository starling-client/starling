//! 2D overlay: one atlas holding GUI sprites, the bitmap font, item and block
//! textures, and a batch of textured quads in GUI pixels (vanilla's automatic
//! GUI scale is applied when drawing).
use crate::{Result, resource_source::ResourceSource};
use smallvec::SmallVec;
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use sunrise_assets::animation::AnimationMetadata;
use wgpu::util::DeviceExt;

use model_reducing::reducible_model_sprites;

mod animate;
pub mod cit_glint;
mod clip_corner;
pub mod dynamic_textures;
mod gui_sprite;
pub use gui_sprite::atlas_key_of_gui_sprite;
mod menu_pictures;
mod model_reducing;
pub(crate) mod pack_text;
pub mod pictures;
pub(crate) mod sprite_modifiers;

/// GUI sprite ids a draw asked for that the atlas lacks, each warned about once.
static MISSING_GUI_SPRITES: std::sync::Mutex<BTreeSet<String>> =
    std::sync::Mutex::new(BTreeSet::new());

#[cfg(test)]
thread_local! {
    static GUI_SPRITE_MISSES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The GUI sprite ids this thread's draws missed since the last call, for tests that a screen names only real sprites.
#[cfg(test)]
pub(crate) fn take_gui_sprite_misses() -> Vec<String> {
    GUI_SPRITE_MISSES.with(|misses| std::mem::take(&mut *misses.borrow_mut()))
}

/// A draw named a GUI sprite the atlas lacks (a wrong name, a pack's filter, a mod's id): one warning per id.
fn missing_gui_sprite(id: &str) {
    #[cfg(test)]
    GUI_SPRITE_MISSES.with(|misses| misses.borrow_mut().push(id.to_owned()));
    let mut warned = MISSING_GUI_SPRITES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if warned.insert(id.to_owned()) {
        log::warn!("no GUI sprite {id:?} in the atlas; drawing the missing texture");
    }
}

/// A sprite's atlas region, and its size in GUI pixels: the size of the
/// default resources' texture of that name, whatever the resolution of the
/// pack that replaced it (vanilla's GUI code draws in those units: `blit`
/// divides by the texture size it is given, sprites by their `gui.scaling`
/// size). Textures the default resources lack keep their own size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub w: u32,
    pub h: u32,
}
impl Rect {
    /// The region's size in texels of an atlas this large.
    pub fn texels(&self, (width, height): (u32, u32)) -> (u32, u32) {
        (
            ((self.u1 - self.u0) * width as f32).round() as u32,
            ((self.v1 - self.v0) * height as f32).round() as u32,
        )
    }
}
/// `TextureMetadataSection`'s `blur` and `clamp`: how vanilla samples a
/// standalone texture (`ReloadableTexture.setSampler`: linear rather than
/// nearest filtering, clamp-to-edge rather than repeat addressing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureFlags {
    pub blur: bool,
    pub clamp: bool,
}
impl TextureFlags {
    /// The `texture` section of a `.png.mcmeta` file, if it sets either flag.
    pub fn from_mcmeta(json: &str) -> Option<Self> {
        let meta: serde_json::Value = crate::packs::first_json(json.as_bytes()).ok()?;
        let section = meta.get("texture")?;
        let flag = |key| section.get(key).and_then(serde_json::Value::as_bool) == Some(true);
        let flags = Self {
            blur: flag("blur"),
            clamp: flag("clamp"),
        };
        (flags != Self::default()).then_some(flags)
    }
}
mod cit;
use cit::*;
mod atlas_sprites;
pub use atlas_sprites::*;
mod atlas;
pub use atlas::*;
mod atlas_pack;
mod image;
pub use image::*;
mod batch;
mod batch_text;
mod renderer;
pub use batch::*;
pub use renderer::*;

#[cfg(test)]
#[path = "hud/coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
mod tests;

//! A client mod's dynamic textures (`ClientRenderClientHost::create_dynamic_texture`,
//! `update_dynamic_texture`, `free_dynamic_texture` and `dynamic_texture_budget`, vanilla's
//! `DynamicTexture` registered in the `TextureManager`): RGBA images kept in the HUD atlas, in a
//! pool of fixed slots of three size classes, within a budget per mod; what a mod has left of it,
//! and why a call was refused.

/// The sides of the pool's three size classes, smallest first: a slot of a class is that many
/// pixels square, and an image takes a slot of the smallest class whose side is at least its
/// width and its height.
pub const DYNAMIC_TEXTURE_CLASSES: [u32; 3] = [64, 256, 1024];
/// The longest side of a dynamic texture, the largest class's: a longer one is
/// [`DynamicTextureError::TooLarge`].
pub const DYNAMIC_TEXTURE_MAX_SIDE: u32 = DYNAMIC_TEXTURE_CLASSES[2];
/// The most dynamic textures one mod holds at once ([`DynamicTextureError::OverBudget`]).
pub const DYNAMIC_TEXTURE_MOD_IMAGES: u32 = 16;
/// The most bytes of pixels (`width * height * 4` an image, whatever its slot) one mod's
/// dynamic textures hold together, 4 MiB ([`DynamicTextureError::OverBudget`]): one
/// 1024 x 1024 image, or sixteen 256 x 256 ones. The game keeps each image's last pixels in
/// memory to upload them again after a resource reload, and this is what they may take.
pub const DYNAMIC_TEXTURE_MOD_BYTES: u64 = 4 << 20;

/// What the calling mod may still create: images and bytes of pixels left of its budget
/// ([`DYNAMIC_TEXTURE_MOD_IMAGES`], [`DYNAMIC_TEXTURE_MOD_BYTES`]). It does not say whether a
/// slot of a class is free: slots are shared by every mod, and a full class is
/// [`DynamicTextureError::NoSlot`]. Only the game makes one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct DynamicTextureBudget {
    /// Images the mod may still create.
    pub images: u32,
    /// Bytes of pixels its images may still add, four a pixel.
    pub bytes: u64,
}

/// Why a dynamic texture call changed nothing. The mod's images, every other mod's and the
/// atlas are as they were before the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DynamicTextureError {
    /// The id is not in a namespace owned by the calling mod (`mymod:dynamic/panel` for the mod
    /// `mymod`, its extra manifest namespaces, or a content package it serves via `served_by`),
    /// or no mod is calling: a mod creates, updates and frees only its own images.
    NotOwned,
    /// A side is 0, or the pixels are not `width * height * 4` bytes.
    BadSize,
    /// A side is longer than [`DYNAMIC_TEXTURE_MAX_SIDE`]: no class of the atlas pool holds the
    /// image. Larger images, as textures of their own, are not part of this API.
    TooLarge,
    /// Every slot of the class the image takes is held, by this mod or others: free one, or
    /// make the image fit a smaller class.
    NoSlot,
    /// The mod would hold more than [`DYNAMIC_TEXTURE_MOD_IMAGES`] images or
    /// [`DYNAMIC_TEXTURE_MOD_BYTES`] of pixels: free some first (`dynamic_texture_budget` says
    /// what is left).
    OverBudget,
    /// `create_dynamic_texture` of an id that already names an image, or a sprite of the GUI
    /// atlas: update it, or choose another id.
    NameTaken,
    /// `update_dynamic_texture` or `free_dynamic_texture` of an id that names no image.
    Unknown,
    /// `update_dynamic_texture` with a size whose class is not the image's: an update re-patches
    /// the slot the image holds, so free the image and create it again at the new size.
    ClassChange,
    /// The atlas has no pool: nothing draws here (a server's thread, a dedicated server, no
    /// window), or no client mod was loaded when the atlas was built.
    Unavailable,
}

impl std::fmt::Display for DynamicTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotOwned => "the id is not in the calling mod's namespace",
            Self::BadSize => "a side is 0 or the pixels are not width * height * 4 bytes",
            Self::TooLarge => "a side is longer than the largest slot class",
            Self::NoSlot => "every atlas slot of the image's class is held",
            Self::OverBudget => "the calling mod's dynamic textures would pass its budget",
            Self::NameTaken => "the id already names an image or a GUI sprite",
            Self::Unknown => "no dynamic texture has this id",
            Self::ClassChange => "the new size needs another slot class than the image's",
            Self::Unavailable => "the HUD atlas has no dynamic texture pool here",
        })
    }
}

impl std::error::Error for DynamicTextureError {}

impl crate::NoBackend for DynamicTextureError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

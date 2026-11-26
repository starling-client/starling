//! A client mod's dynamic textures (`ClientRenderClientHost::create_dynamic_texture`,
//! `update_dynamic_texture`, `update_dynamic_texture_region` and `release_dynamic_texture`,
//! vanilla's `DynamicTexture` registered in the `TextureManager`): each one a GPU texture of its
//! own, `RGBA8_UNORM` (never sRGB), one mip level unless more are asked for, sampled nearest with
//! repeat (`DynamicTexture`'s constructor), drawn by
//! [`GuiCanvas::texture`](crate::GuiCanvas::texture) from level 0. This file holds why a call
//! was refused.

/// Why a dynamic texture call changed nothing. The mod's textures, every other mod's and the
/// GPU's are as they were before the call.
///
/// A surviving variant's wire index moved down when `NoSlot`, `OverBudget` and `ClassChange`
/// were removed; `OutOfBounds` and then `BadMipLevels` were added last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DynamicTextureError {
    /// The id is not in a namespace owned by the calling mod (`mymod:dynamic/panel` for the mod
    /// `mymod`, its extra manifest namespaces, or a content package it serves via `served_by`),
    /// or no mod is calling: a mod creates, updates and releases only its own textures.
    NotOwned,
    /// A side is 0, or the pixel buffer's length is not `width * height * 4` bytes. Never a
    /// panic: the buffer is checked before anything is made.
    BadSize,
    /// A side is longer than the device's largest 2D texture (`max_texture_dimension_2d`). With
    /// `BadMipLevels`, the only limits a dynamic texture has.
    TooLarge,
    /// `create_dynamic_texture` of an id that already names a dynamic texture: update it, or
    /// release it first.
    NameTaken,
    /// `update_dynamic_texture`, `update_dynamic_texture_region` or `release_dynamic_texture`
    /// of an id that names no dynamic texture. It is also what a texture the game lost reads as
    /// (the graphics device was lost and rebuilt, so every texture is gone): create it again.
    Unknown,
    /// Nothing draws here (a server's thread, a dedicated server, no window), so there is no
    /// device to hold a texture.
    Unavailable,
    /// `update_dynamic_texture_region` of a region that is not inside the texture.
    OutOfBounds,
    /// `create_dynamic_texture` of `mip_levels` of 0 or more than the texture's full chain
    /// (`32 - max(width, height).leading_zeros()`), or `update_dynamic_texture_region` of a
    /// `mip_level` the texture does not have, or `update_dynamic_texture` of a size whose full
    /// chain is shorter than the texture's `mip_levels`.
    BadMipLevels,
}

impl std::fmt::Display for DynamicTextureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotOwned => "the id is not in the calling mod's namespace",
            Self::BadSize => "a side is 0 or the pixels are not width * height * 4 bytes",
            Self::TooLarge => "a side is longer than the device's largest texture",
            Self::NameTaken => "the id already names a dynamic texture",
            Self::Unknown => "no dynamic texture has this id",
            Self::Unavailable => "no graphics device holds dynamic textures here",
            Self::OutOfBounds => "the region is not inside the texture",
            Self::BadMipLevels => "the mip level count or mip level is not one the texture has",
        })
    }
}

impl std::error::Error for DynamicTextureError {}

impl crate::NoBackend for DynamicTextureError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

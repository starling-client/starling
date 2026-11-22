//! `ClientRenderClientHost::create_dynamic_texture`, `update_dynamic_texture`,
//! `free_dynamic_texture` and `dynamic_texture_budget`: a client mod's dynamic textures (vanilla's
//! `DynamicTexture` in the `TextureManager`), held in the GUI atlas's pool of slots for the mod
//! whose hook or part is running (`crash::current_native_mod`) and named among the atlas's
//! sprites, so `native_sprite` and every other sprite path draw them.
use super::Game;
use mod_api::{DynamicTextureBudget, DynamicTextureError, Identifier};
use sunrise_client::hud::dynamic_textures::ImagePatch;

impl Game<'_> {
    /// The calling mod's image `id` made from `rgba` (`width * height` pixels of four bytes) in
    /// a slot of the atlas's pool, its pixels uploaded with the next frame.
    pub(in crate::game) fn create_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        let owner = mod_api::content::crash::current_native_mod();
        let (pool, mut names) = self.gui.atlas.dynamic_names();
        self.dynamic_textures
            .create(pool, &mut names, owner, id, [width, height], rgba)
    }
    /// The calling mod's image `id` given new pixels, written into its slot with the next frame.
    pub(in crate::game) fn update_dynamic_texture(
        &mut self,
        id: &Identifier,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), DynamicTextureError> {
        let owner = mod_api::content::crash::current_native_mod();
        let (pool, mut names) = self.gui.atlas.dynamic_names();
        self.dynamic_textures
            .update(pool, &mut names, owner, id, [width, height], rgba)
    }
    /// The calling mod's image `id` freed: its name and slot go at once.
    pub(in crate::game) fn free_dynamic_texture(
        &mut self,
        id: &Identifier,
    ) -> Result<(), DynamicTextureError> {
        let owner = mod_api::content::crash::current_native_mod();
        let (pool, mut names) = self.gui.atlas.dynamic_names();
        self.dynamic_textures.free(pool, &mut names, owner, id)
    }
    /// What the calling mod may still create.
    pub(in crate::game) fn dynamic_texture_budget(
        &self,
    ) -> Result<DynamicTextureBudget, DynamicTextureError> {
        let owner = mod_api::content::crash::current_native_mod();
        self.dynamic_textures
            .budget(self.gui.atlas.dynamic_texture_pool.as_ref(), owner)
    }
    /// The images created, updated or lost by a rebuilt atlas texture since the last frame, as
    /// patches into their slots for this frame's batch.
    pub(in crate::game) fn dynamic_texture_patches(&mut self, patches: &mut Vec<ImagePatch>) {
        self.dynamic_textures
            .patches(self.gui.atlas.dynamic_texture_pool.as_ref(), patches);
    }
}

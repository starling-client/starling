use super::Game;
use mod_api::{HookId, PlayerModel, PlayerSkinOverride};

impl Game<'_> {
    /// Collect the local player's skin and arm width together, before render extraction.
    pub(in crate::game) fn native_resolve_player_skin(
        &self,
        selection: &mut Option<mod_api::PlayerSkinOverride>,
    ) {
        self.fire_resolve_player_skin(selection);
    }

    /// Install the resolved appearance on the render-only avatar and return the original pair.
    pub(in crate::game) fn apply_player_skin_override(
        &mut self,
        panorama: bool,
    ) -> Option<(Option<String>, bool)> {
        if !mod_api::__private::listened(HookId::ResolvePlayerSkin)
            || panorama
            || self.player.is_none()
            || self.player_model.is_none()
        {
            return None;
        }
        let mut selection = None;
        self.native_resolve_player_skin(&mut selection);
        let PlayerSkinOverride { texture, model, .. } = selection?;
        // The same packed atlas is sampled by the body, the hand, and the preview.
        // The pool is absent if the current atlas has no slots for live images.
        if self.gui.atlas.dynamic_texture_pool.is_none()
            || self.dynamic_textures.size(&texture) != Some([64, 64])
        {
            return None;
        }
        let avatar = self.player_model.as_mut()?;
        Some((
            avatar.skin.replace(texture.to_string()),
            std::mem::replace(&mut avatar.slim, model == PlayerModel::Slim),
        ))
    }
}

#[cfg(test)]
#[path = "resolve_player_skin/tests.rs"]
mod tests;

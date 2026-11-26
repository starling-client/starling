use super::Game;
use mod_api::{HookId, Identifier, PlayerModel, PlayerSkinOverride};

/// The atlas skin slot the selected dynamic texture is copied into.
pub(in crate::game) struct PlayerSkinSlot {
    texture: Identifier,
    /// The key the slot is held under in `skin_slots`.
    location: String,
    slot: usize,
    /// The slot's sprite name, `sunrise/skin{slot}`.
    name: String,
    /// The slot's texel origin in the atlas ([`HudAtlas::skin_origins`]).
    origin: [u32; 2],
    /// [`HudAtlas::revision`] it was resolved against.
    atlas: u64,
}

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
        // The body, the hand and the preview sample one packed atlas, so the 64 x 64 dynamic
        // texture is copied on the GPU into a skin slot of it, every frame it is selected
        // (as `skin_slots` hands slots out for downloaded skins). Anything else keeps vanilla.
        if self.dynamic_textures.size(&texture) != Some([64, 64]) {
            return None;
        }
        // The slot, its sprite name and its texel origin are made again only when the selection
        // changes, the slot is given to another skin, or the atlas is rebuilt.
        let held = self.player_skin_slot.as_ref().is_some_and(|held| {
            held.texture == texture
                && held.atlas == self.gui.atlas.revision
                && self.skin_slots.slot(&held.location) == Some(held.slot)
        });
        if !held {
            let location = format!("dynamic:{texture}");
            let (slot, _) = self.skin_slots.place(location.clone())?;
            let origin = *self.gui.atlas.skin_origins.get(slot)?;
            self.player_skin_slot = Some(PlayerSkinSlot {
                texture: texture.clone(),
                location,
                slot,
                name: format!("sunrise/skin{slot}"),
                origin,
                atlas: self.gui.atlas.revision,
            });
        }
        let held = self.player_skin_slot.as_ref()?;
        let (name, origin) = (held.name.clone(), held.origin);
        self.dynamic_textures.copy_to_atlas(&texture, origin);
        let avatar = self.player_model.as_mut()?;
        Some((
            avatar.skin.replace(name),
            std::mem::replace(&mut avatar.slim, model == PlayerModel::Slim),
        ))
    }
}

#[cfg(test)]
#[path = "resolve_player_skin/tests.rs"]
mod tests;

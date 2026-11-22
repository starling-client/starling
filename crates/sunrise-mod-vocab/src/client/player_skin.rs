use super::entity_layers::PlayerModel;
use crate::Identifier;

/// The local player's appearance for the frame, selected by `resolve_player_skin`.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct PlayerSkinOverride {
    /// The identifier of a live 64×64 RGBA image uploaded with `create_dynamic_texture`.
    pub texture: Identifier,
    /// The arm width and skin layout used to draw the texture.
    pub model: PlayerModel,
}

impl PlayerSkinOverride {
    /// Select `texture` with `model`. The renderer looks up the texture after all
    /// listeners have answered; if it is unavailable, both vanilla values are kept.
    pub fn new(texture: Identifier, model: PlayerModel) -> Self {
        Self { texture, model }
    }
}

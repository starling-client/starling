//! The light a mob is drawn with.
use super::Game;
use sunrise_client::mobs::Mob;

impl Game<'_> {
    /// `EntityRenderer.getPackedLightCoords`: the light `mob` is drawn with at `alpha`.
    pub(in crate::game) fn mob_packed_light(&self, mob: &Mob, alpha: f64) -> u32 {
        sunrise_client_gameplay::mob_light::packed_light(&self.world, mob, alpha)
    }
}

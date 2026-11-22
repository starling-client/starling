//! The light a mob is drawn with.
use sunrise_client::mobs::{Mob, Species};
use sunrise_client::world::World;

/// `EntityRenderer.getPackedLightCoords`: the light `mob` is drawn with at `alpha`, sky in byte 0, block in byte 1.
pub fn packed_light(world: &World, mob: &Mob, alpha: f64) -> u32 {
    // The probe is the mob's eye at the position this frame draws, not this tick's.
    let probe = mob.light_probe(alpha);
    let light = world.light(probe[0], probe[1], probe[2]);
    let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
    // EntityRenderer.getBlockLightLevel: burning mobs and the Allay, Blaze, Magma Cube, Vex, Wither get block light 15.
    if mob.fire_ticks > 0 && !mob.mod_fire_immune
        || matches!(
            mob.species(),
            Species::Allay | Species::Blaze | Species::MagmaCube | Species::Vex | Species::Wither
        )
    {
        packed = (packed & 0xFF) | ((15 * 16) << 8);
    }
    // GlowSquidRenderer.getBlockLightLevel: the body brightens as the dark ticks run out.
    if mob.species() == Species::GlowSquid {
        let lit = (packed >> 12) & 15;
        let block = sunrise_client::mobs::aquatic::glow_squid_block_light(mob, lit);
        packed = (packed & 0xFF) | ((block * 16) << 8);
    }
    packed
}

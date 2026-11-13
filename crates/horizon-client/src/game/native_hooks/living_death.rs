use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A mob or server player is about to die (NeoForge's `LivingDeathEvent`,
    /// vanilla's `LivingEntity.die`): its health has run out, and the loot,
    /// experience and death message have not run. `entity` is its id and
    /// `kind` its type (`minecraft:zombie`, `minecraft:player`); `damage_type`
    /// names the last blow (`minecraft:player_attack`) and `attacker` is the
    /// mob that dealt it, 0 for none or a player. Set `cancel` to keep it alive:
    /// Horizon decides death by health, so it stays at 1 health. The hook
    /// fires once per death attempt. Armor stands, boats and other non-mob
    /// entities are not told. API 392.
    pub(in crate::game) fn native_living_death(
        &mut self,
        entity: u32,
        kind: &str,
        damage_type: &str,
        attacker: u32,
        cancel: &mut bool,
    ) {
        self.fire_living_death(entity, kind, damage_type, attacker, cancel);
    }
}

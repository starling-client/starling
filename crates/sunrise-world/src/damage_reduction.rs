//! Living damage-source bypass, armor and resistance/protection dispatch.
use crate::{
    combat_rules::{self, ArmorWorld},
    living_damage::{DamageContainer, Reduction, STAT_LIMIT, java_max_f32, stat_amount},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bypass {
    Armor,
    Effects,
    Resistance,
    Enchantments,
}
pub trait DamageReductionWorld: ArmorWorld {
    fn bypasses(&mut self, kind: Bypass) -> Result<bool, Self::Error>;
    fn hurt_armor(&mut self, damage: f32) -> Result<(), Self::Error>;
    fn armor_value(&mut self) -> Result<i32, Self::Error>;
    fn toughness_attribute(&mut self) -> Result<f64, Self::Error>;
    fn has_resistance(&mut self) -> Result<bool, Self::Error>;
    fn resistance_amplifier(&mut self) -> Result<i32, Self::Error>;
    fn victim_is_server_player(&self) -> bool;
    fn award_resisted(&mut self, amount: i32) -> Result<(), Self::Error>;
    fn attacker_is_server_player(&mut self) -> Result<bool, Self::Error>;
    /// Performs the second native attacker lookup before awarding its stat.
    fn award_dealt_resisted(&mut self, amount: i32) -> Result<(), Self::Error>;
    fn damage_protection(&mut self, level: Self::ServerLevel) -> Result<f32, Self::Error>;
}
pub fn armor_from_attribute(value: f64) -> i32 {
    value.floor() as i32
}
pub fn after_armor<W: DamageReductionWorld>(world: &mut W, damage: f32) -> Result<f32, W::Error> {
    if world.bypasses(Bypass::Armor)? {
        return Ok(damage);
    }
    world.hurt_armor(damage)?;
    let armor = world.armor_value()? as f32;
    let toughness = world.toughness_attribute()? as f32;
    combat_rules::after_armor(world, damage, armor, toughness)
}
/// `getDamageAfterMagicAbsorb`: Resistance, then protection enchantments, applying any
/// registered reduction modifiers at NeoForge's guarded `setReduction` sites.
pub fn after_magic<W: DamageReductionWorld>(
    world: &mut W,
    container: &mut DamageContainer,
    mut damage: f32,
) -> Result<f32, W::Error> {
    if world.bypasses(Bypass::Effects)? {
        return Ok(damage);
    }
    // NeoForge patches setReduction, not the method's local vanilla damage. Protection
    // still runs against that local; its reduction is measured against the live container.
    let mut live = damage;
    if world.has_resistance()? && !world.bypasses(Bypass::Resistance)? {
        let reduction = world
            .resistance_amplifier()?
            .wrapping_add(1)
            .wrapping_mul(5);
        let remainder = 25i32.wrapping_sub(reduction);
        let original = damage;
        damage = java_max_f32((damage * remainder as f32) / 25.0, 0.0);
        let resisted = original - damage;
        if resisted > 0.0 && resisted < STAT_LIMIT {
            live = container.reduce(Reduction::MobEffects, original, resisted, damage);
            if world.victim_is_server_player() {
                world.award_resisted(stat_amount(resisted))?;
            } else if world.attacker_is_server_player()? {
                world.award_dealt_resisted(stat_amount(resisted))?;
            }
        } else if !container.uses_live_reductions() {
            live = damage;
        }
    }
    if damage <= 0.0 {
        return Ok(if container.uses_live_reductions() {
            live
        } else {
            0.0
        });
    }
    if world.bypasses(Bypass::Enchantments)? {
        return Ok(live);
    }
    let protection = if let Some(level) = world.server_level()? {
        world.damage_protection(level)?
    } else {
        0.0
    };
    if protection > 0.0 {
        let after = combat_rules::after_magic(damage, protection);
        let nominal = live - after;
        let unmodified = if live.to_bits() == damage.to_bits() {
            after
        } else {
            java_max_f32(live - nominal, 0.0)
        };
        live = container.reduce(Reduction::Enchantments, live, nominal, unmodified);
    }
    Ok(live)
}

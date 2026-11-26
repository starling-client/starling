//! Inputs and refusals for a living entity's death-table roll.

use crate::{EntityRef, Identifier, StackWriteError};

/// The `DamageSource` for [`EntityRef::roll_death_loot`].
// no wire: a borrowed call-scoped argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LootDamageSource<'a> {
    /// The registered damage type, such as `minecraft:player_attack`.
    pub damage_type: &'a Identifier,
    /// `DamageSource.getEntity`: the attacker, for example an arrow's shooter.
    pub causing: Option<EntityRef>,
    /// `DamageSource.getDirectEntity`: the arrow in a ranged attack, or the attacker itself in melee.
    pub direct: Option<EntityRef>,
}

impl<'a> LootDamageSource<'a> {
    /// Describe a source without dealing damage. For melee, pass the attacker for both
    /// `causing` and `direct`; for environmental damage without an entity, pass neither.
    pub const fn new(
        damage_type: &'a Identifier,
        causing: Option<EntityRef>,
        direct: Option<EntityRef>,
    ) -> Self {
        Self {
            damage_type,
            causing,
            direct,
        }
    }
}

/// Why [`EntityRef::roll_death_loot`] could not return the rolled stacks.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum DeathLootError {
    /// No server backend can roll here, or the level's loot tables are unavailable.
    Unavailable,
    /// A supplied handle is stale or unavailable to this host.
    StaleEntity,
    /// The target is not a `LivingEntity`.
    NotLiving,
    /// The world's damage type registry has no entry with that ID.
    UnknownDamageType,
    /// The named or default death loot table is absent.
    UnknownTable,
    /// The roll succeeded, but a generated stack could not cross into the mod API.
    Stack(StackWriteError),
}

impl std::fmt::Display for DeathLootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "death loot is unavailable on this host",
            Self::StaleEntity => "a death loot entity handle no longer resolves here",
            Self::NotLiving => "only a living entity has a death loot context",
            Self::UnknownDamageType => "the damage type is not registered in this world",
            Self::UnknownTable => "the death loot table is not loaded",
            Self::Stack(_) => "a rolled stack could not be represented in the mod API",
        })
    }
}

impl std::error::Error for DeathLootError {}

impl crate::NoBackend for DeathLootError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

impl From<StackWriteError> for DeathLootError {
    fn from(error: StackWriteError) -> Self {
        Self::Stack(error)
    }
}

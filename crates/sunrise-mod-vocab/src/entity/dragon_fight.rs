/// A stage of the End's dragon respawn (`DragonRespawnStage`), in vanilla's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, sunrise_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum DragonRespawnStage {
    /// The four crystals on the exit portal aim at its centre (`START`).
    Start,
    /// The crystals' beams rise over the portal (`PREPARING_TO_SUMMON_PILLARS`).
    PreparingToSummonPillars,
    /// Each pillar in turn is rebuilt with its crystal (`SUMMONING_PILLARS`).
    SummoningPillars,
    /// The beams meet over the portal (`SUMMONING_DRAGON`).
    SummoningDragon,
    /// The portal crystals are gone and a new dragon flies (`END`).
    End,
}

/// The state of a level's dragon fight (`EnderDragonFight`), read with
/// `LevelRef::dragon_fight`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct DragonFightState {
    /// No dragon of the fight lives (`dragonKilled`): the fight is won, or no
    /// dragon was made yet.
    pub won: bool,
    /// A dragon of this fight was killed before (`hasPreviouslyKilledDragon`).
    pub previously_won: bool,
    /// The respawn under way (`respawnStage`), or None.
    pub respawn_stage: Option<DragonRespawnStage>,
    /// The crystals on the pillars at the last count (`aliveCrystals`), which
    /// the fight makes every 100 ticks while its dragon lives and when a
    /// crystal is destroyed.
    pub alive_crystals: i32,
}

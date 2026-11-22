//! The `level_rules` area: game rules, difficulty and the features a level enables. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `level_rules` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelRulesBackend {
    /// Side: server.
    /// The difficulty (`Level.getDifficulty`): `peaceful`, `easy`, `normal`
    /// or `hard`, as `/difficulty` names it.
    fn level_difficulty(&self, level: LevelRef) -> Option<&'static str>;
    /// Side: server.
    /// The local difficulty at `pos` (`ServerLevel.getCurrentDifficultyAt`), as mobs spawned
    /// there are finalized with: the world difficulty scaled by the level's age, the chunk's
    /// inhabited time and the moon (both 0 for a chunk that isn't loaded). Its
    /// `special_multiplier` is vanilla's `getSpecialMultiplier`. None for a stale handle, a level
    /// the server has not opened, or a client.
    fn level_local_difficulty(&self, level: LevelRef, pos: [i32; 3]) -> Option<Difficulty>;
    /// Side: server.
    /// Read a registered boolean game rule.
    fn level_game_rule_bool(&self, level: LevelRef, key: BoolGameRuleKey) -> Option<bool>;
    /// Side: server.
    /// Read a registered integer game rule (`GameRules.getInt`), always within the range it was
    /// declared with. None for a stale handle, a level the server has not opened, or a client.
    fn level_game_rule_int(&self, level: LevelRef, key: IntGameRuleKey) -> Option<i32>;
    /// Side: server.
    /// Game rule `rule` (`minecraft:keep_inventory` or `keep_inventory`), as
    /// `/gamerule` queries it: `Ok(None)` for an unknown rule. A gone/reopened level
    /// returns `Stale`; an unreadable side or phase returns `Unavailable`.
    fn level_game_rule(
        &self,
        level: LevelRef,
        rule: &str,
    ) -> Result<Option<GameRuleValue>, LevelReadError>;
    /// Side: server.
    /// Every game rule the world has (`GameRules`) with its current value, by `minecraft:` id
    /// (`max_minecart_speed` only with minecart improvements). A gone/reopened level
    /// returns `Stale`; an unreadable side or phase returns `Unavailable`.
    fn level_game_rules(
        &self,
        level: LevelRef,
    ) -> Result<Vec<(String, GameRuleValue)>, LevelReadError>;
    /// Side: server.
    /// `LevelReader.enabledFeatures` of the level a handle names. Every level of a world shares
    /// the world's flags. None for a stale handle and on a client.
    fn level_enabled_features(&self, level: LevelRef) -> Option<FeatureFlags>;
}
/// The handles' writes of the `level_rules` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait LevelRulesBackendMut {
    /// Side: server.
    /// Sets game rule `rule` (`minecraft:keep_inventory` or `keep_inventory`) to `value`, as
    /// `/gamerule <rule> <value>` does (`GameRules.set` and `MinecraftServer.onGameRuleChanged`):
    /// the world follows it (`advance_time` freezes the clocks), `game_rule_changed` fires with
    /// no actor, and the players' clients get the rules they show. The rules are the world's,
    /// so any open level's handle sets them. True also when the rule already had the value.
    /// False for an unknown rule, a value of the other kind or outside the rule's range, a
    /// stale handle and a client.
    fn level_set_game_rule(&mut self, level: LevelRef, rule: &str, value: GameRuleValue) -> bool;
}

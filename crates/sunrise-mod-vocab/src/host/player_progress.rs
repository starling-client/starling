//! The `player_progress` area: statistics and criteria triggers. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_progress` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerProgressBackend {
    /// Side: server.
    /// The value of statistic `stat_type` for `key` (`minecraft:stone`,
    /// `minecraft:jump`...), as `ServerStatsCounter.getValue`: 0 when never
    /// counted. Reads the player's own counter even during a nested turn that
    /// displaces them. In `player_stat_award` the award has not been applied yet.
    fn player_stat(&self, player: PlayerRef, stat_type: StatType, key: &str) -> Option<i32>;
}
/// The handles' writes of the `player_progress` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerProgressBackendMut {
    /// Side: server.
    /// Fires the mod's criteria trigger `trigger` (`mymod:pipe_connected`)
    /// for `player`: vanilla's
    /// `SimpleCriterionTrigger.trigger`. Each criterion of theirs with that
    /// trigger, not yet granted, of an advancement not yet done, is tested
    /// by `test` and then by its `player` predicate; those that pass are
    /// granted, and an advancement they complete is awarded with its toast,
    /// chat line and rewards, as the game's own triggers award.
    ///
    /// The acting player is reached inside their hook; another live player
    /// is reached in their own level, including during a guest's turn. An
    /// outer player displaced by a nested turn is unavailable. `Ok(())`
    /// means the player was reached, whether or not anything was granted.
    /// A stale/offline/displaced handle, a remote client, or a `minecraft:`
    /// trigger or empty id is refused with a typed reason.
    ///
    /// `test` is lent to this call only. A failed test stops further testing
    /// and reports `Test`; earlier matching criteria may already be granted.
    fn player_trigger_criterion(
        &mut self,
        player: PlayerRef,
        trigger: &Identifier,
        test: Arc<dyn CriterionTest>,
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server. Fires only `default_block_use`, without a tool.
    /// The engine fires `any_block_use` once when the enclosing block use consumes.
    /// It sees the hand after the use; a replacement stack is visible, unlike vanilla's original object.
    /// Evaluates vanilla predicates and rewards in the player's own level; never reads inventory.
    /// `Ok(())` means reached, even if no criterion matched. Refuses remote clients
    /// (`NotServer`), stale/offline/displaced players (`PlayerUnavailable`), and
    /// unloaded positions (`UnloadedChunk`).
    fn player_trigger_block_use(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server. Fires only `item_used_on_block` with `original`.
    /// `original` is copied after a before-block PASS and before block/item effects.
    /// The stack is not read from inventory. The engine fires `any_block_use`
    /// once when the enclosing block use consumes, including a consuming cancel.
    /// Call this helper only when the mod also intends the inner criterion.
    /// An invalid stack returns `Stack(StackWriteError)` before the trigger fires.
    /// Evaluates vanilla predicates and rewards in the player's own level.
    /// `Ok(())` means reached, even if nothing matched. Refuses remote clients
    /// (`NotServer`), stale/offline/displaced players (`PlayerUnavailable`), and
    /// unloaded positions (`UnloadedChunk`).
    fn player_trigger_item_used_on_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        original: &ItemStack,
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server.
    /// Vanilla `CriteriaTriggers.CONSUME_ITEM.trigger(serverPlayer, stack)`, also public
    /// to NeoForge mods. Fires only `minecraft:consume_item` through the food-eating
    /// criterion path, without consuming a stack, awarding a used statistic or reading inventory.
    /// Predicates and rewards run in the player's own level. `Ok(())` means reached,
    /// even if nothing matched. Refuses clients (`NotServer`), stale/offline/displaced
    /// players (`PlayerUnavailable`) and invalid stacks (`Stack(StackWriteError)`)
    /// before firing.
    fn player_trigger_consume_item(
        &mut self,
        player: PlayerRef,
        stack: &ItemStack,
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server.
    /// `Player.awardStat(stat, amount)`: adds `amount`, as the game does
    /// for its own counts, so `player_stat_award` hears it and may rewrite or cancel
    /// it, and scoreboard criteria follow. False for a player it can't
    /// reach.
    fn player_award_stat(
        &mut self,
        player: PlayerRef,
        stat_type: StatType,
        key: &str,
        amount: i32,
    ) -> bool;
    /// Side: server.
    /// `Player.resetStat`: sets the statistic back to 0, which scoreboard
    /// criteria follow. False for a player it can't reach.
    fn player_reset_stat(&mut self, player: PlayerRef, stat_type: StatType, key: &str) -> bool;
}

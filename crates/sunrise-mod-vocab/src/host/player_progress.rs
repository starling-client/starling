//! The `player_progress` area: statistics, advancement progress and criteria triggers. Host method declarations; access defaults and game forwarders are generated.
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
    /// Side: server.
    /// An owned snapshot of the player's live counters in the `ServerStatsCounter.toJson`
    /// save format: namespaced keys grouped by statistic type and the current `DataVersion`.
    /// A reachable player without counters has an empty `stats` map. Returns `None` on
    /// a client or for a stale or unavailable player.
    fn player_statistics(&self, player: PlayerRef) -> Option<String>;
    /// Side: server.
    /// An owned snapshot of the player's live progress on advancement `id`, mirroring
    /// `ServerPlayer.getAdvancements().getOrStartProgress(holder)`.
    ///
    /// A loaded advancement the player has never touched answers with every criterion
    /// unobtained (`obtained_at: None`), `completed_requirements == 0`, and `done == false`.
    /// Reads live state rather than the save file, including inside
    /// `player_advancement_progress` and `player_advancement_done` (after the criterion
    /// change) and when a nested player turn has displaced the player.
    ///
    /// Returns `None` for an unknown advancement id, a stale, offline or unavailable
    /// player, or a client host, and until the engine has loaded the player's
    /// advancements, which happens by the player's first tick.
    fn player_advancement_progress(
        &self,
        player: PlayerRef,
        id: &Identifier,
    ) -> Option<AdvancementProgress>;
}
/// The handles' writes of the `player_progress` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerProgressBackendMut {
    /// Side: server.
    /// Awards an advancement or one of its criteria to `player`, mirroring
    /// `PlayerAdvancements.award(holder, criterion)` (and `AdvancementCommands.Action.GRANT`
    /// when `criterion` is `None`).
    ///
    /// With `Some(name)`, grants that criterion. With `None`, grants every remaining
    /// criterion if the advancement is not yet done. Routes through the same engine path
    /// as triggers and `/advancement grant`, so completion grants rewards, announces in chat
    /// according to the advancement's display and `show_advancement_messages`, shows the
    /// toast, fires `player_advancement_progress` and `player_advancement_done`, and queues
    /// the client's progress update.
    ///
    /// Inside a transaction, validation runs immediately and the returned `bool` reports
    /// whether the change would alter current progress, while the change itself—along with
    /// its rewards, messages, hooks and client updates—happens when the outermost
    /// transaction commits and is dropped if it aborts; until then reads see the old
    /// progress.
    ///
    /// Returns `Ok(true)` when progress changed (or would change inside a transaction) and
    /// `Ok(false)` when nothing changed (such as an already-obtained criterion or an
    /// already-done advancement with `None`). Refuses clients (`NotServer`), stale, offline,
    /// dead, displaced or unreachable players (`PlayerUnavailable`), unknown advancements
    /// (`UnknownAdvancement`), and unknown criterion names (`UnknownCriterion`).
    fn player_award_advancement(
        &mut self,
        player: PlayerRef,
        id: &Identifier,
        criterion: Option<&str>,
    ) -> Result<bool, AdvancementError>;
    /// Side: server.
    /// Revokes an advancement or one of its criteria from `player`, mirroring
    /// `PlayerAdvancements.revoke(holder, criterion)` (and `AdvancementCommands.Action.REVOKE`
    /// when `criterion` is `None`).
    ///
    /// With `Some(name)`, revokes that criterion. With `None`, revokes every obtained
    /// criterion. Routes through the same engine path as `/advancement revoke`, firing
    /// `player_advancement_progress` for each revoked criterion and queueing the client's
    /// progress update.
    ///
    /// Inside a transaction, validation runs immediately and the returned `bool` reports
    /// whether the change would alter current progress, while the change itself—along with
    /// its messages, hooks and client updates—happens when the outermost transaction
    /// commits and is dropped if it aborts; until then reads see the old progress.
    ///
    /// Returns `Ok(true)` when progress changed (or would change inside a transaction) and
    /// `Ok(false)` when nothing was obtained. Refuses clients (`NotServer`), stale, offline,
    /// dead, displaced or unreachable players (`PlayerUnavailable`), unknown advancements
    /// (`UnknownAdvancement`), and unknown criterion names (`UnknownCriterion`).
    fn player_revoke_advancement(
        &mut self,
        player: PlayerRef,
        id: &Identifier,
        criterion: Option<&str>,
    ) -> Result<bool, AdvancementError>;
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
    /// is reached in their own level. An outer player displaced by a nested turn or a player
    /// whose level could not be entered is unavailable. `Ok(())` means the player was reached,
    /// whether or not anything was granted. A malformed
    /// id is refused, as is an exact built-in trigger key (which uses its own event matcher).
    /// A namespace by itself does not reserve a key.
    ///
    /// `test` and the parsed conditions are borrowed for this call only. A failed test stops
    /// further tests in this invocation; earlier matching criteria may already be granted.
    /// A panic is logged and shown in an error toast; later invocations may lend another test.
    fn player_trigger_criterion(
        &mut self,
        player: PlayerRef,
        trigger: &Identifier,
        test: &mut dyn CriterionTest,
    ) -> Result<(), TriggerCriterionError>;
    /// Side: server. Fires only `default_block_use`, without a tool.
    /// The engine fires `any_block_use` once when the enclosing block use consumes.
    /// It sees the hand after the use; a replacement stack is visible, unlike vanilla's original object.
    /// Evaluates vanilla predicates and rewards in the player's own level; never reads inventory.
    /// `Ok(())` means reached, even if no criterion matched. Refuses clients (`NotServer`) and
    /// unreachable players or levels (`PlayerUnavailable`). The game resolves the event block
    /// as vanilla does (`BlockUnavailable` on read failure); separate location predicates
    /// check the chunks they need themselves.
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
    /// `Ok(())` means reached, even if nothing matched. Refuses clients (`NotServer`) and
    /// unreachable players or levels (`PlayerUnavailable`). The event carries the actual block
    /// state and optional block entity (`BlockUnavailable` on state read failure). The position
    /// need not already be resident; predicates that read other locations check them themselves.
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
    /// even if nothing matched. Refuses clients (`NotServer`), unreachable players or levels
    /// (`PlayerUnavailable`) and invalid stacks (`Stack(StackWriteError)`)
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
    /// Side: server.
    /// Replaces the player's live counters with borrowed save JSON, upgrading it as
    /// `ServerStatsCounter.parse` does. An absent `stats` map means empty; decode errors
    /// are logged and recoverable registered entries are retained. Returns `false`
    /// without changing the player if the document is malformed, no map can be decoded,
    /// or the player is stale or unavailable for writes (including dead or displaced).
    ///
    /// Omitted counters read as zero. Changed statistic scores follow the replacement,
    /// and changed counts, including zero for removals, go out on the next statistics
    /// request. The normal player save persists the replacement. No `player_stat_award`
    /// hook fires. Inside a transaction the replacement, with its scores, happens when the
    /// outermost transaction commits and is dropped if it aborts; until then reads see the
    /// old counters.
    fn player_set_statistics(&mut self, player: PlayerRef, snapshot: &str) -> bool;
}

//! The `on_destroyed_by_player` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player removes the block `state` at `pos` (`IBlockExtension.onDestroyedByPlayer`, called
    /// from NeoForge's `ServerPlayerGameMode.destroyBlock` as the removal), with `tool`, a copy of
    /// the stack in the player's hand taken before the tool wore (`None` for a bare hand), and
    /// `will_harvest`, whether the break will drop loot (never in creative). `state` is the state
    /// `playerWillDestroy` left, which has already run (a pot's cracking, piglin anger, a
    /// tripwire's disarming, a creative shulker box's or beehive's drop). `will_harvest` is the
    /// one harvest answer of the break: the tool's `hasCorrectToolForDrops` with the mods' harvest
    /// listeners (`can_harvest_block`, then `player_harvest_check`) already applied, asked once
    /// before the tool wore, as NeoForge's `canHarvestBlock`. Asked after `break_block` allowed
    /// the break and after the tool wore, so the stack in the hand is no longer `tool`.
    /// `destroyed` is whether the block was actually destroyed: `Some(true)` says the part
    /// removed it itself, `Some(false)` that it stays (a block of layers losing one layer: change
    /// it with the `host` and leave it standing), and either answer replaces the game's own
    /// removal. `Some(false)` ends the break there: no drops or statistic, though the tool has
    /// worn. Left `None`, vanilla's removal runs. Server-side, for every player of the world; the
    /// first part answering wins. Asked of the block's own parts only, so a block with none costs
    /// nothing.
    ///
    /// Version 2: `playerWillDestroy` runs before the part, and no longer only when none answers;
    /// `will_harvest` is the final answer including the mods' harvest listeners; `tool` is the
    /// pre-wear copy; the tool wears before the removal, not after.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "replace",
        out = "destroyed",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.onDestroyedByPlayer",
        version = 2
    )]
    fn on_destroyed_by_player(
        player: u128,
        pos: [i32; 3],
        state: BlockState,
        tool: Option<&ItemStack>,
        will_harvest: bool,
        destroyed: &mut Option<bool>,
    );
}

//! The `item_smelted` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `FurnaceResultSlot.checkTakeAchievements` ran at a furnace, blast
    /// furnace or smoker's result slot with a nonzero `removeCount` (NeoForge
    /// `PlayerEvent.ItemSmeltedEvent`): `player` is the live player handle,
    /// `stack` lends the stack vanilla passed there (`getSmelting`), and
    /// `amount_removed` is `removeCount` (`getAmountRemoved`). On the server it
    /// fires with `HostSide::Server` after the furnace awards its used recipes
    /// and pops experience (`AbstractFurnaceBlockEntity.awardUsedRecipesAndPopExperience`);
    /// on the client it fires with `HostSide::Client` during predicted menu
    /// clicks without awarding experience. A pickup or throw passes the taken
    /// stack and its count; a shift-click fires once with the pre-move stack
    /// and the count moved (`onQuickCraft`); a hotbar swap has
    /// `removeCount == 0` so the server awards experience without firing
    /// `item_smelted`. Both logical sides.
    #[hook(
        dispatch = "api_carrier",
        side = "both",
        contract = "listen",
        neoforge = "PlayerEvent.ItemSmeltedEvent",
        cancellable = false,
        version = 2
    )]
    fn item_smelted(player: PlayerRef, stack: ItemStackView<'_>, amount_removed: u32);
}

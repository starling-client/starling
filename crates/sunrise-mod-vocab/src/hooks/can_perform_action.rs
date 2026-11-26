//! The `can_perform_action` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.canPerformAction`: can `stack` perform the item
    /// ability `ability`, a plain, case-sensitive name such as `shield_block` or
    /// `sword_sweep` (a mod's own abilities also use plain names, such as
    /// `my_mod_chop`; `host.item_abilities()` lists the game's)?
    /// `can_perform` arrives with vanilla's answer for that ability: the item kind's
    /// (shears shear), its tag's (`#minecraft:swords` sweep) or the stack's component
    /// (`blocks_attacks`, a shield's, blocks); set it either way and the stack grants or
    /// loses the ability. `stack` is a borrowed [`ItemStackView`] (call
    /// [`ItemStackView::project`] only when you need components or an owned copy). The
    /// answer depends only on the stack and the ability: the hook is asked from gameplay
    /// and from loot predicates (`neoforge:can_item_perform_ability`) and must give the
    /// same answer in both. So it has no host or level, by design, as NeoForge's has
    /// none; the [`ItemAbilityContext`] lends only the merged tags (`context.tags()`).
    /// Behaviour that depends on the world belongs in the hook for that moment (block
    /// breaking, item use). Loot conditions use the shared item defaults. Asked where
    /// vanilla checks the item for an action: a raised item blocking attacks
    /// (`shield_block`: a granted stack blocks as a bare
    /// `blocks_attacks` component does, a denied one never blocks nor is raised);
    /// shearing a sheep, mooshroom, snow golem, copper golem or bogged, cutting a
    /// mob's, boat's or knot's leads, a dispenser's shears and a bee nest's
    /// honeycombs (`shears_harvest`); carving a pumpkin (`shears_carve`); shearing a
    /// mob's saddle, carpet, harness, body block or wolf armor off
    /// (`shears_remove_armor`); cropping a growing plant's tip (`shears_trim`);
    /// disarming a broken tripwire (`shears_disarm`); a mined block's wear as shears
    /// wear (`shears_dig`); brushing an armadillo (`brush_brush`); priming TNT and
    /// lighting a campfire, candle or candle cake, by hand or from a dispenser
    /// (`firestarter_light`); putting out a campfire (`shovel_douse`); a cast line
    /// staying out and drawn from the hand, and which hand another player's cast rod
    /// shows in (`fishing_rod_cast`); a trident's throw
    /// (`trident_throw`, a trident's own use, so only a denial counts); a charged
    /// grounded hit sweeping (`sword_sweep`); and the spyglass's zoom, overlay, view
    /// and held-to-the-eye pose (`spyglass_scope`, of the item in use). It decides
    /// whether the stack may do the action, not what the action does to the world or the
    /// tool (`block_tool_modification` is the block's side of a tool action; item wear
    /// is its own). Only the `ItemPart`s attached to `stack`'s item are asked, on both
    /// sides, so the client's prediction agrees with the server: answer from the stack
    /// and `ability` alone, and fast. An item with no such part costs one table lookup
    /// and does not project the stack.
    /// A client without this mod predicts vanilla here; the server corrects it.
    #[hook(
        receiver = "associated",
        dispatch = "api_carrier",
        side = "both",
        contract = "listen",
        context = "item_ability",
        scope = "item",
        key = "stack",
        version = 2,
        neoforge = "IItemExtension.canPerformAction"
    )]
    fn can_perform_action(stack: ItemStackView<'_>, ability: &str, can_perform: &mut bool);
}

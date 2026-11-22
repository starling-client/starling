//! The `pre_remove_side_effects` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockEntity.preRemoveSideEffects` of a mod's block entity: an
    /// authoritative server is removing `entity` from `pos`, where the block
    /// `state` stood, and its updates may have side effects (not under flag
    /// 256, `UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS`). Put in `drops` the stacks
    /// it held that the game doesn't: they scatter as `Containers.dropItemStack`
    /// scatters them, after the block entity's own container. The block entity
    /// is still in place, so its attachments read as they were. Answered from
    /// inside the level's write: don't change the world here. This also runs
    /// when a menu button callback removes the backing block through the Host.
    /// Set `keep_contents` to skip the base container slots. Each `drops` entry
    /// is a slot: `None` draws the empty slot's scatter position but spawns nothing.
    /// Then `spawns` creates each (stack, position) with default pickup delay,
    /// and `experience` awards each (amount, position) as experience orbs.
    /// Each output list accepts at most 1024 entries; invalid positions or stacks
    /// refuse the outputs. Positions must be finite. Default outputs draw nothing.
    #[notes = r#"- `pre_remove_side_effects` is a mod block entity's `BlockEntity.preRemoveSideEffects`: its block is being
  replaced or broken with side effects (not under flag 256). Its part puts the stacks it held in `drops`, and they
  scatter as `Containers.dropItemStack` scatters them, after the block entity's own container slots.
  `drops` uses `Option<ItemStack>` entries: `None` consumes an empty slot's three position draws.
  `keep_contents` skips the base container slots; then `spawns` creates (stack, position) entries
  with default pickup delay, and `experience` awards (amount, position) entries as orbs.
  Each list is limited to 1024 entries and positions must be finite. It runs inside
  the level's write with the level host, like `neighbor_changed`; the block entity is still in place, so the part
  reads its attachments with `TYPE.get(host, Holder::BlockEntity(pos))`, the level host being their store. A part that returns nothing draws nothing.
  A ticking or menu-hook (`menu_button`, `menu_quick_move`, `menu_slot_take`) block entity stays in the level:
  removing its block through the Host runs the usual removal, and the old scope can no longer lend a view.
  Version 4 includes removal from a menu callback; do not drop the same contents manually there."#]
    #[hook(
        side = "server",
        contract = "collect",
        out = "drops",
        version = 4,
        scope = "block_entity",
        key = "entity",
        target = "sunrise_client::game::native_hooks::pre_remove_side_effects::GameLevel<'_>::native_pre_remove_side_effects",
        neoforge = "none (vanilla BlockEntity.preRemoveSideEffects; NeoForge adds no event or extension for it)"
    )]
    fn pre_remove_side_effects(
        pos: BlockPos,
        state: BlockState,
        entity: &ModBlockEntity,
        keep_contents: &mut bool,
        drops: &mut Vec<Option<ItemStack>>,
        spawns: &mut Vec<(ItemStack, [f64; 3])>,
        experience: &mut Vec<(u32, [f64; 3])>,
    );
}

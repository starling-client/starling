//! The `on_block_placed_by` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockItemExtension.onBlockPlacedBy`: the selected block has been placed,
    /// its final block state and the original item's block-entity data applied, and
    /// `Block.setPlacedBy` completed (including companion blocks). Runs once,
    /// at the primary position, before the item's placement sound, game event,
    /// placement trigger and stack consumption. `state` is the live block state
    /// after the callback, which can have changed the block or run its own effects.
    /// `stack` is the pre-shrink stack; `placer` is the placing player, or None
    /// for a dispenser. `level` is the level the block was placed in. `face` and `click` are the placement context's clicked
    /// face and world-space hit point. Failed, refused and canceled placements
    /// never notify, nor do successful writes that leave a different block at the
    /// primary position. Only Parts attached to the original item hear this server notice.
    #[notes = r##"### After a block item places its block

`ItemPart::on_block_placed_by` is the server-side `BlockItemExtension.onBlockPlacedBy`
notice (hook version 3). It runs on the original item's Part, after the selected
block's final state, stack block-entity data and `Block.setPlacedBy` work exist,
including a bed's head, at the primary position
(the bed's foot). The event carries the pre-shrink `stack`, optional `placer`, placement `level`, `pos`,
live `state` after the callback, clicked `face` and world-space `click`. A dispenser has no placer.
Use the `ServerHost` and block-entity handles to copy stack data or link companions;
writes are not overwritten by the placement's stack-data application. The notice
precedes the item's placement sound, game event, trigger and consumption, and
cannot cancel the placement. The block callback can run its own effects first,
including clearing a completed wither pattern. A refusal, placement failure,
`entity_place` cancellation or primary write that leaves a different block before
`setPlacedBy` never fires it. Items without a
subscribed Part cost one `item_listens` lookup and no stack conversion.
"##]
    #[hook(
        side = "server",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "BlockItemExtension.onBlockPlacedBy",
        version = 3
    )]
    fn on_block_placed_by(
        stack: &ItemStack,
        placer: Option<PlayerRef>,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockState,
        face: BlockFace,
        click: [f64; 3],
    );
}

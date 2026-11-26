//! The `entity_place` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockEvent.EntityPlaceEvent` (and `EntityMultiPlaceEvent`, which extends it): `placer` places `state` at `pos` against `placed_against`. For `BlockItem.place` and mob placement (an enderman's carried block or a snow golem's trail), the hook fires before the block write and before an item stack shrinks; `None` is reserved for a placement with no entity. `placement` is `Single`, or `Multi` with every block the placement writes (a bed's foot and head, a door's halves, a tall plant), where `state` and `pos` stay the primary block (the foot, the lower half). For players, `placed_against` is the state adjacent to the primary placement position opposite the clicked face, including when the clicked block is replaceable. For mobs it is the state below the placement. `PlayerRef::place_block` also fires this hook before commit: `placer` is that player, `placed_against` uses the supplied face, and the level already holds the tentative states and block entities, including every cell written by `Block.setPlacedBy`. Its `Multi` lists each written cell's staged state. Cancel stops the placement: no block is committed, the stack is kept and an enderman keeps carrying its block. Server-side. Diverges from upstream: replaces the listen-only `place_block`; a mod that stopped placements by claiming `native_use_on` cancels here.
    /// A failed listener's journaled writes are discarded; a panic does not veto the placement or stop later listeners.
    #[hook(dispatch = "flat", side = "server", contract = "cancel", out = "canceled", neoforge = ["BlockEvent.EntityPlaceEvent", "BlockEvent.EntityMultiPlaceEvent"], cancellable = true, version = 2, listener_transaction = "when_open")]
    fn entity_place(
        placer: Option<EntityRef>,
        pos: BlockPos,
        state: BlockState,
        placed_against: BlockState,
        placement: Placement,
        canceled: &mut bool,
    );
}

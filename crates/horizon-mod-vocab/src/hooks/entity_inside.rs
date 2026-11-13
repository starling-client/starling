//! The `entity_inside` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity is inside a block state this part is attached to (`Block.entityInside`). `event.precise` is `Entity.checkInsideBlocks`' flag: the entity moved nearly a block, or its box at the target touches the block. Fires for mobs, projectiles, dropped items and players on the server, once per entity and block per move, after the entity-kind batch (or the player's turn) has ticked, not mid-sweep; a contact whose entity is gone or whose block changed is dropped. Vanilla's `InsideBlockEffectApplier` is not exposed.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        neoforge = "none (vanilla Block.entityInside; NeoForge adds no event or extension for it)"
    )]
    fn entity_inside();
}

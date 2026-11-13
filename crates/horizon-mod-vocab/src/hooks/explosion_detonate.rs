//! The `explosion_detonate` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0042

pub trait Hooks {
    /// An explosion at `center` in `dimension` (`minecraft:overworld`) is
    /// about to break `blocks`: `ServerExplosion.explode` with its `toBlow`,
    /// before any block is hit. Take out the positions to spare; they keep
    /// their block and get no fire. Entities are hurt as before. Server-side.
    #[hook(
        side = "server",
        contract = "collect",
        out = "blocks",
        neoforge = "ExplosionEvent.Detonate"
    )]
    fn explosion_detonate();
}

//! The `fire_odds` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0215

pub trait Hooks {
    /// On an authoritative server, how fire treats the block `state`
    /// (`FireBlock.setFlammable(block, igniteOdds, burnOdds)`): put
    /// `(ignite, burn)` in `odds` for your own blocks, and vanilla's table
    /// isn't read. Ignite odds above zero let fire stand beside the block and
    /// spread into the air around it (planks 5, wool 30, leaves 30); burn odds
    /// are how fast fire destroys it (planks 20, wool 60, leaves 60). `(0, 0)`
    /// never burns; keep both within vanilla's 0 to 100. A waterlogged state
    /// never burns and isn't asked. Answer only for your own blocks and leave
    /// `odds` empty otherwise; the level is read-only. Asked hundreds of times
    /// per fire tick: keep it short.
    #[hook(
        side = "server",
        contract = "replace",
        out = "odds",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_fire_odds",
        neoforge = "IBlockExtension.getFlammability, IBlockExtension.getFireSpreadSpeed"
    )]
    fn fire_odds();
}

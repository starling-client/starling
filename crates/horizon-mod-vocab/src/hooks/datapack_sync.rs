//! The `datapack_sync` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0123

pub trait Hooks {
    /// The server is about to send its data to the players (NeoForge's
    /// `OnDatapackSyncEvent`, its `sendRecipes`): push the recipe types
    /// (`minecraft:smelting`, `mymod:crusher`) whose recipes your client half
    /// needs. Horizon sends their final recipes — the world's data packs
    /// merged, load conditions held — to every Horizon client when it joins
    /// and again after each `/reload`; a vanilla client is sent nothing new.
    /// Asked once per load, when the server's data tables are built, and the
    /// same types go to every player: unlike NeoForge, which asks again for
    /// each joining player, it can't differ between players. Push nothing and
    /// nothing is sent; push nothing after a load that pushed some and the
    /// clients go back to their own recipes. Server-side (the single
    /// player's integrated server included).
    #[hook(
        side = "server",
        contract = "collect",
        out = "recipe_types",
        neoforge = "OnDatapackSyncEvent"
    )]
    fn datapack_sync();
}

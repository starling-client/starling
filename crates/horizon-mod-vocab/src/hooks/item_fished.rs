//! The `item_fished` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player `player` (UUID) and `player_entity_id` reeled in a
    /// catch (`ItemFishedEvent`, in `FishingHook.retrieve` after the fishing
    /// loot rolled): `fished` holds the bobber and the items, and
    /// `rod_damage` the damage the rod takes (vanilla's 1, or 2 with the
    /// bobber on the ground), which the mods may set (`damageRodBy`, not
    /// negative). Set `cancel` and the player receives nothing: no items,
    /// experience or statistic, and no `fishing_rod_hooked` trigger; the
    /// bobber is gone and the rod
    /// still takes `rod_damage`. After `player_fish`'s reel-in. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "ItemFishedEvent",
        cancellable = true
    )]
    fn item_fished();
}

//! The `recipe_sources` hook: declarations read by the mod API generator.
pub trait Hooks {
    /// Additional ingredient containers for server player `player` placing or checking a
    /// recipe in the mod crafting menu `menu` (`namespace:path`) at `pos`. The player's
    /// state is not swapped in. Push block positions to `sources`; containers are used
    /// after the player's inventory, in mod order and then push order. The engine silently
    /// skips `pos`, duplicates (first wins), and unavailable or non-lendable containers.
    /// No separate container-count cap applies.
    /// Chest-like block entities and mod containers with declared slots are lendable.
    /// No answer preserves vanilla placement. The engine takes ingredients and writes
    /// changed containers back, firing `container_changed`; this hook only names sources.
    /// Asked once per recipe-book placement/apply or server transfer check, not per tick.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "sources",
        version = 2,
        neoforge = "none (Sunrise mod crafting menus lend ingredient containers)",
        cancellable = false
    )]
    fn recipe_sources(player: u128, menu: &str, pos: [i32; 3], sources: &mut Vec<[i32; 3]>);
}

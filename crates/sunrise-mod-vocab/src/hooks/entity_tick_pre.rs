//! The `entity_tick_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0154

pub trait Hooks {
    /// The entities `entities` are about to tick in the level `dimension`, and `kinds` holds
    /// the type of each (`minecraft:zombie`, `minecraft:item`), in the same order (NeoForge's
    /// `EntityTickEvent.Pre`, posted right before each entity's tick in `tickNonPassenger`).
    /// Told on the server for the mobs, players, item entities, minecarts, boats, general entity
    /// rows and projectiles that tick in an entity-ticking chunk, and the entities the server
    /// keeps without simulating them (displays, markers, interactions), whose tick is only told
    /// (not a player's own boat or cart, which ticks with the player); and on a client for the
    /// mobs, other players, projectiles, tridents and displays it ticks; `host` says which side.
    /// Minecarts, boats and general rows are each told alone. Eligible types named at
    /// registration with `entity_tick_each` (or requested through `entity_tick_each_entity`)
    /// are also told alone. Other observed entities are told once per family. On the server,
    /// player Pre runs before the observed parts of `ServerPlayer.tick`, not the connection
    /// body's `player_tick_pre`. The local client player is not yet observed here.
    /// Push an ID from `entities` onto `canceled` to skip that entity's observed tick work and
    /// suppress its Post. IDs outside this call are ignored; repeated IDs count once. A mod
    /// using these hooks also opts in through `entity_tick_listened`.
    /// Removing a general entity during its own Pre skips its tick and Post and dismounts its
    /// passengers unless dismount is vetoed. An eligible former passenger can still tick as a
    /// root in this pass if it has not ticked and its join-order slot is still ahead. An
    /// already-passed slot is not revisited.
    /// Version 3 expands the version 2 coverage without changing the event signature.
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_entity_tick_pre",
        dispatch = "flat",
        side = "both",
        version = 3,
        contract = "collect",
        out = "canceled",
        neoforge = "EntityTickEvent.Pre",
        cancellable = true
    )]
    fn entity_tick_pre(dimension: &str, entities: &[u32], kinds: &[&str], canceled: &mut Vec<u32>);
}

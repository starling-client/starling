//! The `entity_size` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity's dimensions are being resolved for `pose` (NeoForge's `EntityEvent.Size`,
    /// fired from vanilla's `Entity.refreshDimensions` and from the entity's constructor): it is
    /// new, its pose changed to `pose`, it grew up, its `minecraft:scale` changed, or another value
    /// its vanilla size follows changed; or as a player's pre-motion candidate (version 2, below).
    /// `size` (width, height and eye height, in blocks) starts as vanilla's new size and
    /// `old_size` is the size it had. Change `size` and the entity's box and eyes take it; later
    /// mods see earlier mods' edits. The answer holds while the entity keeps that pose and
    /// vanilla size; the next change asks again, starting from vanilla's. Changing the width or
    /// height leaves the eye height as it is: set it too. Zero-size answers are valid. An answer
    /// is ignored if any component is non-finite, negative or greater than 32 blocks. This bounds
    /// collision and pathfinding work while allowing sizes larger than naturally occurring
    /// vanilla entities. `host` reads the side that asks; the entity may be new, so read only
    /// what you need of it.
    ///
    /// Every entity asks, of every kind, on whichever side runs that refresh: the server for its
    /// own entities, and a client for its copies of the server's: when one is added, when the
    /// server sends a new pose (even one with the same size), and when `minecraft:scale` or
    /// another synched value the server sends changes its vanilla size (vanilla's
    /// `Entity.onSyncedDataUpdated`). Each side asks for its own copy and
    /// keeps its own answer: answer from the event, the same way on both, so that the server's
    /// box and the client's agree. On the server, a mob is asked before and after its tick, and
    /// every other entity (an item, a minecart, a boat, a projectile, an armor stand, TNT, a
    /// falling block, an orb, a rocket, a trident, a display, a marker, an interaction, an end
    /// crystal...) at the same points, standing, the first time from its constructor; its box
    /// (collisions, block contacts, picking) takes the answer. A player is asked in two steps:
    /// before the player's motion, for the candidate poses its fit tests use (on a client for
    /// its own player, and once a tick for each LAN guest's player),
    /// and after the motion, for the current pose's change (a client's own player after its
    /// tick). A size changed within a tick takes the answer at the next of those. Not
    /// cancellable.
    ///
    /// Version 2: the player's standing, crouching and swimming pose fit tests also use
    /// this answer, resolved before motion with live scale; an unchanged current-pose
    /// answer is reused, while other candidates are asked once per player tick.
    /// Selection and application share the answer, including cloned motion snapshots.
    /// A scale change during movement, or a replay with a different vanilla-size key,
    /// uses vanilla for that fit decision until the next pre-motion resolution.
    /// Uncached fall-flying, sleeping and spin-attack candidates keep vanilla's box.
    /// Candidate resolution dispatches 2–3 times per player tick with subscribers, never without.
    ///
    /// Version 3: a client's copies of other entities ask too, and their box, picking and hitbox
    /// outline take the answer; version 2 kept vanilla's size for them. Every kind of entity
    /// asks, not only mobs and players.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 3,
        contract = "collect",
        out = "size",
        host = "read",
        neoforge = "EntityEvent.Size",
        cancellable = false
    )]
    fn entity_size(entity: EntityRef, pose: mod_api::Pose, old_size: [f32; 3], size: &mut [f32; 3]);
}

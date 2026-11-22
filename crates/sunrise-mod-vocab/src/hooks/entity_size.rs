//! The `entity_size` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An entity's dimensions are being resolved for `pose`: after a pose or vanilla-size change
    /// (NeoForge's `EntityEvent.Size`, vanilla's `Entity.refreshDimensions`: its pose changed to
    /// `pose`, it grew up, its `minecraft:scale` changed, or it is new), or as a player's
    /// pre-motion candidate (version 2, below). `size` (width, height and eye height, in
    /// blocks) starts as vanilla's new size and `old_size` is the size it had. Change `size`
    /// and the entity's box and eyes take it; later mods see earlier mods' edits. The answer
    /// holds while the entity keeps that pose and vanilla size; the next change asks again,
    /// starting from vanilla's. Changing the width or height leaves the eye height as it is:
    /// set it too. Zero-size answers are valid. An answer is ignored if any component is
    /// non-finite, negative or greater than 32 blocks. This bounds collision and pathfinding
    /// work while allowing sizes larger than naturally occurring vanilla entities. `host`
    /// reads the side that asks; the entity may be new, so read only what you need of it.
    ///
    /// A mob is asked on the server before and after its tick. A player is asked in two steps:
    /// before the player's motion, for the candidate poses its fit tests use (on a client for
    /// its own player, and once a tick for each LAN guest's player), and after the motion, for
    /// the current pose's change (a client's own player after its tick). A size changed within
    /// a tick takes the answer at the next of those. Does not fire for:
    /// other kinds of entity, whose vanilla sizes are fixed, and a client's copies of other
    /// entities, which keep vanilla's size. Not cancellable.
    ///
    /// Version 2: the player's standing, crouching and swimming pose fit tests also use
    /// this answer, resolved before motion with live scale; an unchanged current-pose
    /// answer is reused, while other candidates are asked once per player tick.
    /// Selection and application share the answer, including cloned motion snapshots.
    /// A scale change during movement, or a replay with a different vanilla-size key,
    /// uses vanilla for that fit decision until the next pre-motion resolution.
    /// Uncached fall-flying, sleeping and spin-attack candidates keep vanilla's box.
    /// Candidate resolution dispatches 2–3 times per player tick with subscribers, never without.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 2,
        contract = "collect",
        out = "size",
        host = "read",
        neoforge = "EntityEvent.Size",
        cancellable = false
    )]
    fn entity_size(entity: EntityRef, pose: mod_api::Pose, old_size: [f32; 3], size: &mut [f32; 3]);
}

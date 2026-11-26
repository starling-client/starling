//! The `baby_entity_spawn` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Two animals bred and the baby they made (if any) is about to join the world
    /// (NeoForge's `BabyEntitySpawnEvent`, vanilla's `Animal.spawnChildFromBreeding`
    /// and `Fox.FoxBreedGoal.breed`): `dimension` is the level they are in
    /// (a replacement baby's `PendingMobSpec` names it), `parent` and `partner` are entity
    /// ids, `player` and `player_entity_id` the UUID and entity id of the
    /// same player in this level who fed them into love (`getCausedByPlayer`): the
    /// initiating parent's resolved feeder first, then the partner's, or both `None`.
    /// When breeding constructed a baby (a mule for a horse and a donkey), it already
    /// has its inheritance drawn, but is not finalized or in any level: it sits in
    /// the single optional breeding slot `child` (`BabyEntitySpawnChild<'_>`). If
    /// breeding produced no initial child, the hook still fires with an empty slot
    /// that a listener may populate.
    /// Inspect the slot (`event.child.get()`), edit the child in place
    /// (`event.child.get_mut()`), construct a replacement
    /// (`event.child.construct(host, spec)?`) and swap it in (`event.child.replace(child)?`),
    /// or remove it (`event.child.take()` / `event.child.clear()`). Any detached
    /// [`UnspawnedMob`] constructed by a listener fires `entity_constructing` immediately.
    /// A detached child not left in the slot is cleanly discarded when dropped.
    /// If a listener panics, the shared dispatcher restores its cancellation flag and
    /// disables that listener for the session. Direct writes to the child and slot are
    /// not journaled, so edits made before the panic remain visible to later listeners.
    /// A replacement must be an `AgeableMob` (`minecraft:fox` for fox breeding), but need
    /// not start as a baby. This hook runs inside the breeding goal or brain behavior, before
    /// either parent's cooldown and before the initiating animal's movement and age/love tick.
    /// On acceptance, ordinary animals set the child's baby age and snap it to the
    /// initiating parent's live position with zero yaw and pitch before awarding credit.
    /// Foxes grant breeder trust and award credit using the child's pre-snap age and position,
    /// then reset the parents before setting baby age and snapping the child. Both paths
    /// insert the selected child once; its state remains unpublished through the ordinary
    /// breeding experience orb's earlier admission.
    /// Set `cancel` (or return `Flow::Cancel`) to abort breeding: matching NeoForge's
    /// `Animal.spawnChildFromBreeding` and `Fox.FoxBreedGoal.breed`, both parents are
    /// reset to age 6000 and in-love 0, breeding credit is cleared, and there is no
    /// baby, no experience, no `animals_bred` statistic, no advancement and no heart
    /// particles. By contrast, returning `Flow::Continue` with an empty slot skips
    /// spawning and finalization without resetting the parents or consuming credit.
    /// The inheritance draws the construction made stay spent.
    /// Not fired for turtles, frogs and sniffers (an egg, no baby), or their hatching;
    /// armadillos take the ordinary animal path and are included. Villagers do not post
    /// this animal-only event, matching NeoForge's documented `Animal.spawnChildFromBreeding`
    /// posting site; a rejected villager join instead leaves the child's bed free. On the server.
    #[notes = r#"## Baby entity spawn

- **`baby_entity_spawn(dimension, parent, partner, player, player_entity_id, child, cancel)`** (NeoForge's `BabyEntitySpawnEvent`, version 4): fires after breeding has attempted to construct the baby (`Animal.spawnChildFromBreeding` / `Fox.FoxBreedGoal.breed`) and before breeding snaps the baby, finalizes breeding, and inserts it. Even when no initial baby was constructed, the hook fires with an empty slot. `child` (`BabyEntitySpawnChild<'_>`) lends the single optional breeding slot directly on the stack: inspect the current baby with `event.child.get()`, edit it in place with `event.child.get_mut()`, construct a new baby with `event.child.construct(host, PendingMobSpec::new(kind, dimension, position, yaw, true))?` and swap it in with `event.child.replace(baby)?`, or clear the slot with `event.child.take()` / `event.child.clear()`. Creating a detached child fires `entity_constructing` immediately; discarding it cleans up its unpublished state on drop. A replacement must be an `AgeableMob` (`minecraft:fox` in fox breeding), but may start as an adult. Invalid replacements answer `Invalid`. The event runs before either parent's breeding cooldown.
- A panicking listener is disabled for this hook by the shared dispatcher. Its cancellation flag is restored, but direct writes to the child and slot are not journaled and remain visible to later listeners.
- Cancelling (`Flow::Cancel`) resets both `parent` and `partner` to age 6000 and love 0, clears breeding credit, and spawns no baby, experience, `animals_bred` statistic, advancement or hearts. Returning `Flow::Continue` with an empty slot spawns no baby and skips breeding finalization without resetting the parents or consuming credit (matching NeoForge's `Animal.spawnChildFromBreeding` and `Fox.FoxBreedGoal.breed`). The inheritance draws already made stay spent.
- On acceptance, ordinary animals set the child's baby age and snap it to the initiating parent's live position with zero yaw and pitch before awarding credit. Foxes grant breeder trust and award credit using the child's pre-snap age and position, then reset the parents before setting baby age and snapping the child. Both paths insert the selected child once.
- Turtle and frog breeding, sniffer egg laying and turtle hatching make eggs, not babies, and do not fire it. Armadillos breed through the ordinary animal path and do. Villagers breed through their own behavior and do not (a separate moment)."#]
    #[hook(
        dispatch = "api_carrier",
        side = "server",
        version = 4,
        contract = "cancel",
        out = "cancel",
        neoforge = "BabyEntitySpawnEvent",
        cancellable = true
    )]
    fn baby_entity_spawn(
        dimension: &str,
        parent: u32,
        partner: u32,
        player: Option<u128>,
        player_entity_id: Option<i32>,
        child: BabyEntitySpawnChild<'_>,
        cancel: &mut bool,
    );
}

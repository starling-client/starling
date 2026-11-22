//! The `collision_shape` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The collision shape of the block state `state` at `pos` is being read for the collision
    /// query `query` (vanilla's `BlockBehaviour.getCollisionShape(state, level, pos, context)`),
    /// by an entity's move, a placement check or a projectile's path. `query` is vanilla's
    /// `EntityCollisionContext`, captured when the query was made: the entity's id and class,
    /// descending, placement, always-fluid, the main-hand item, and `is_above`. `shape`
    /// starts as `None`, which keeps the shape the block would otherwise have (its declared one,
    /// a context-keyed one, or the one its block entity published); a part that answers
    /// `Some(boxes)` replaces it for this query, as block-local `[min_x, min_y, min_z, max_x,
    /// max_y, max_z]` boxes. An answer with more than 64 boxes, a box that is not finite or has a
    /// minimum above its maximum is refused. A part that is refused, panics or traps keeps the
    /// normal shape, the block's `collision_shape` parts, those of every mod that attaches one to it,
    /// are disabled for the session and the game logs it once, naming the block and its mods. It is asked only for blocks with a part that answers it, on the thread
    /// that ticks the entity, once for each distinct question (state, position, entity and the rest of the query) in a tick: a step-up retry or a
    /// second sweep reads the first answer. Fluid passage and face-support checks never ask. It
    /// is given the loop `context` of the side that ticks the entity and takes no `Host`: the
    /// part answers from `query` and the state it keeps itself, keyed by the entity's id and
    /// synced to the client, which asks the same question for its own prediction. A collision
    /// query made inside the callback gets the empty-context answer, never a nested call.
    /// Diverges from upstream: new hook.
    #[notes = r#"## Collision shape by the querying entity

- **Block Part method `collision_shape(context, event)`** (`CollisionShapeEvent { pos, state, query, shape }`): answer `event.shape = Some(boxes)` to give the block state `event.state` at `event.pos` another collision shape for this query (vanilla's `BlockBehaviour.getCollisionShape` with its `CollisionContext`; NeoForge adds nothing to it). `event.query` is a `CollisionQuery`: vanilla's `EntityCollisionContext` as it was when the query was made (`entity()` the entity's id, `kind()`, `is_descending()`, `is_placement()`, `always_collide_with_fluid()`, `is_above(shape, pos, default)`, `is_holding_item(item)`, `can_stand_on_fluid(above, fluid)`). `None`, the starting value, keeps the shape the block would otherwise have: its declared shape, a context-keyed one, or the one its block entity published (`shapes_from_block_entity`); `Some` replaces whichever applies for this query only. Boxes are block-local `[min_x, min_y, min_z, max_x, max_y, max_z]`; more than 64 boxes, a coordinate that is not finite or a minimum above its maximum is refused (the bound of a published shape). A refused answer or a panic keeps the normal shape, switches off every `collision_shape` part on that block, whichever mod attached it, for the session (in memory, never saved) and is logged once, naming the block and its mods.
- **Who asks, and when.** Entity movement (`Entity.move` and its step-up retries), placement checks, projectile paths and the other reads that take a `CollisionContext`; never fluid passage or face support. A part is asked once for each distinct question in a tick (the state, the position, and every input of the query), and later reads of that tick (a step-up retry, a second sweep) get the same answer, so a shape cannot change in the middle of a move. A collision query made inside the callback gets the empty-context answer (the block's normal shape), never a nested call. The per-state shape cache stays the empty-context answer, so a block still counts as a large shape where its normal shape is one. It runs on the thread that owns the level, the server tick and the client's own prediction; any other thread (the mesher, world generation) reads the normal shape.
- **Why there is no `Host`.** A move holds the level's collision data and cannot lend a `Host` to a mod in the middle of it, as for `entity_move`, `friction` and the other loop-context hooks. The part answers from the query and from state it keeps itself: a rule that reads more than the query (Create's belt asks whether the entity is in the belt's passenger map; powder snow reads a fall distance) tracks that state in the mod, keyed by `query.entity()`, and syncs it so the client computes the same answer. If a real mod needs an entity read it cannot track itself, one captured field is added to `CollisionQuery`; the `Host` is not reopened.
- **The client's duty.** A client that disagrees with the server about the mod's own state predicts a different shape; the server then corrects the player's position (the usual "moved wrongly" correction, not a kick). Sync the state the rule reads.
- **Cost.** A block with no part that answers it costs nothing. An opted-in block costs one call per distinct question in a tick. Diverges from upstream: new hook."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "block",
        key = "state",
        cancellable = false,
        target = "sunrise_client_lib::collision_parts::native_collision_shape",
        neoforge = "none (vanilla BlockBehaviour.getCollisionShape with its CollisionContext; NeoForge adds no event or extension for it)"
    )]
    fn collision_shape(
        pos: BlockPos,
        state: BlockState,
        query: &CollisionQuery,
        shape: &mut Option<Vec<[f64; 6]>>,
    );
}

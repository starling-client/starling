//! The `mob_spawn_checks` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `MobSpawnEvent.SpawnPlacementCheck`: may the entity type `kind`
    /// spawn at `pos` in `level` for `reason`? This server hook runs after
    /// `SpawnPlacements.checkSpawnRules` evaluates the type's spawn predicate.
    /// `default_result` is the effective predicate result, including predicates composed
    /// or replaced through `Registrar::register_spawn_placement`. It can differ from
    /// vanilla even for a vanilla entity type. `result` starts as `Default`, which keeps
    /// that answer. Set `Succeed` or `Fail` to override it for this check. The predicates
    /// have already drawn from `event.random`, the caller's live stream (NeoForge's
    /// `SpawnPlacementCheck.getRandom()`). Each listener borrows that same stream;
    /// replacing its local reference cannot redirect later listeners. Draws advance
    /// the stream even if the result stays `Default` or the spawn is later refused.
    ///
    /// It runs at `SpawnPlacements.checkSpawnRules`: natural spawning, chunk-generation
    /// packs, zombie reinforcements, spawners and trial spawners. Spawners check the resolved
    /// entity type before creation, including non-mobs. Patrols and village sieges call their
    /// vanilla predicates directly, bypassing both registration and this hook. Ordinary spawners
    /// with custom spawn rules skip it; trial spawners check their custom rules afterward.
    /// `/summon` and spawn eggs do not check placements.
    /// `level` (also named by `dimension`) is the level the entity would spawn in. `spawner`
    /// is `Spawner::Block` or `Spawner::Trial` if a spawner block asked, or `None` otherwise,
    /// including a spawner minecart. The hook lends a read-only host. Listeners should be
    /// pure functions of their inputs because the checks run again on replayed ticks.
    ///
    /// Beyond NeoForge: one `SpawnCheck` answers this hook and `mob_spawn_position_check`;
    /// NeoForge gives them separate enums with the same three constants.
    #[notes = r#"## Spawn checks

- **`mob_spawn_placement_check(dimension, kind, reason, pos, default_result, level, spawner, random, result)`** (NeoForge's `MobSpawnEvent.SpawnPlacementCheck`): at `SpawnPlacements.checkSpawnRules`, `default_result` is the effective predicate result, including predicates composed or replaced through `Registrar::register_spawn_placement`. Set `result` to `SpawnCheck::Succeed` or `Fail` to override it, or leave `Default` to keep it. Each listener borrows the same live `random` stream the predicates already drew from. Reassigning the reference is local to that listener; writes through `result` carry to the next listener. Neither the predicate draws nor the listener draws are undone if the result is overridden or the spawn is refused later. The hook runs for natural spawning, chunk-generation packs, reinforcements, and the type check at spawners and trial spawners, including non-mobs. Patrols and village sieges call vanilla predicates directly, so neither registered predicates nor this hook run there. An ordinary spawner with custom rules skips it; trial spawners check their custom rules afterward.
- **`mob_spawn_position_check(dimension, kind, reason, pos, level, spawner, mob, result)`** (NeoForge's `MobSpawnEvent.PositionCheck`): this hook stands in for `Mob.checkSpawnRules` and `checkSpawnObstruction`: with no listener or `Default` they run and draw as vanilla, while `Succeed` and `Fail` skip them and their draws. The position check also runs for chunk generation, and only replaces obstruction for trial spawners. It gets `mob` (`UnspawnedMobRef`), the mob itself: vanilla creates it before this check, and Sunrise creates it early only when the hook has a listener and hands that same mob on to the spawn. A refused check drops the mob and gives its provisional id back. Creating it draws nothing from the level's random, so `Default` leaves every draw as an unlistened position check's. Its reads are `finalize_spawn`'s (`kind`, `position`, `equipment`, `attribute_base`, `attribute_value`, `tags`, `custom_name`), with no editor. Where the site does not create the mob fresh, the handle answers `kind` and `position` and `Unsupported` for the other reads: chunk generation's packs and a spawner or trial spawner with entity data in its spawn data.
- Both get the read-only host, `level` (`LevelRef`, the level the mob would join) and `spawner` (`Option<Spawner>`: `Block { pos }` for a spawner block, `Trial { pos }` for a trial spawner, `None` otherwise, including a spawner minecart). Both run on replayed ticks, so listeners should be pure functions of their inputs. One `SpawnCheck` serves both (NeoForge's two enums have the same constants)."#]
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "MobSpawnEvent.SpawnPlacementCheck",
        cancellable = false,
        event_per_listener = true,
        version = 2
    )]
    fn mob_spawn_placement_check(
        dimension: &str,
        kind: &str,
        reason: SpawnReason,
        pos: BlockPos,
        default_result: bool,
        level: LevelRef,
        spawner: Option<Spawner>,
        random: &mut dyn RandomSource,
        result: &mut SpawnCheck,
    );
    /// NeoForge's `MobSpawnEvent.PositionCheck`: may the freshly created mob of type `kind` stand at `pos` (its position, a mob's centre) in `level` for `reason`? Vanilla's two checks are `Mob.checkSpawnRules` (the type's own rules) and `Mob.checkSpawnObstruction` (inside blocks or fluids, an ocelot's grass and sea level). `result` starts as `Default`, which runs them; `Succeed` or `Fail` answers instead and they are skipped. It runs on natural spawning, chunk generation's creature packs and spawner blocks (where a spawner with custom spawn rules skips `Mob.checkSpawnRules` on `Default`, as vanilla does).  A trial spawner runs only `checkSpawnObstruction`, so only that is skipped. `spawner` is the block that asked (`Spawner::Block` or `Spawner::Trial`), `None` for natural spawning, chunk generation and a spawner minecart. `mob` is the mob itself, in no level yet: read it through the host (`mob.kind(host)`, `mob.position(host)`, `mob.equipment(host, slot)`); after the hook returns the handle answers `Expired`. It is the mob that spawns if the check passes, handed on rather than created again, and dropped without trace if it fails. Where the game does not create the mob fresh (chunk generation's packs, a spawner whose spawn data carries entity data) the handle answers its kind and position and `Unsupported` for the rest. Server-side. With no listener or `Default` the checks and their random draws are vanilla's (creating the mob for the listener draws nothing from the level's random); `Succeed` and `Fail` skip the checks and so their draws, and the level random's order changes only then; a listener should be a pure function of its inputs, because ticks are replayed.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "MobSpawnEvent.PositionCheck",
        cancellable = false
    )]
    fn mob_spawn_position_check(
        dimension: &str,
        kind: &str,
        reason: SpawnReason,
        pos: [f64; 3],
        level: LevelRef,
        spawner: Option<Spawner>,
        mob: UnspawnedMobRef,
        result: &mut SpawnCheck,
    );
}

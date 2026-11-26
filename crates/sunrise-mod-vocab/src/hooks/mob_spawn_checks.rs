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
    /// `level` (also named by `dimension`) is the level the entity would spawn in. Like NeoForge's
    /// `SpawnPlacementCheck`, this hook carries no spawner identity — inspect `reason` instead.
    /// The hook lends a read-only host. Listeners should be pure functions of their inputs
    /// because the checks run again on replayed ticks.
    ///
    /// Beyond NeoForge: one `SpawnCheck` answers this hook and `mob_spawn_position_check`;
    /// NeoForge gives them separate enums with the same three constants.
    #[notes = r#"## Spawn checks

- **`mob_spawn_placement_check(dimension, kind, reason, pos, default_result, level, random, result)`** (NeoForge's `MobSpawnEvent.SpawnPlacementCheck`): at `SpawnPlacements.checkSpawnRules`, `default_result` is the effective predicate result, including predicates composed or replaced through `Registrar::register_spawn_placement`. Set `result` to `SpawnCheck::Succeed` or `Fail` to override it, or leave `Default` to keep it. Each listener borrows the same live `random` stream the predicates already drew from. Reassigning the reference is local to that listener; writes through `result` carry to the next listener. Neither the predicate draws nor the listener draws are undone if the result is overridden or the spawn is refused later. The hook runs for natural spawning, chunk-generation packs, reinforcements, and the type check at spawners and trial spawners, including non-mobs. Patrols and village sieges call vanilla predicates directly, so neither registered predicates nor this hook run there. An ordinary spawner with custom rules skips it; trial spawners check their custom rules afterward. Like NeoForge's `SpawnPlacementCheck`, it has no spawner field.
- **`mob_spawn_position_check(dimension, kind, reason, pos, level, spawner, mob, result)`** (NeoForge's `MobSpawnEvent.PositionCheck`, version 3): `Default` runs `Mob.checkSpawnRules` and `checkSpawnObstruction` as vanilla; `Succeed` and `Fail` skip them and their draws. This fires for natural mobs, chunk-generation packs and `BaseSpawner` ticks (monster spawner blocks and spawner minecarts), including spawners with configured entity data. Trial spawners run their obstruction check directly. `spawner` (`Option<Spawner>`) is `Spawner::Block { pos }` for a monster spawner block, `Spawner::Minecart { entity }` for a spawner minecart, and `None` otherwise (`MobSpawnEvent.PositionCheck.getSpawner`). `mob` (`UnspawnedMobView<'_>`) borrows the actual new or loaded candidate. Read it directly (`kind`, `position`, `equipment`, `attribute_base`, `attribute_value`, `tags`, `custom_name`); there is no editor or `Expired` handle. A refusal drops the candidate; on success, finalization and admission use the same mob. Creating it does not draw from the level's random.
- Both get the read-only host and `level` (`LevelRef`, the level the entity would join). Both run on replayed ticks, so listeners should be pure functions of their inputs. One `SpawnCheck` serves both (NeoForge's two enums have the same constants)."#]
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "MobSpawnEvent.SpawnPlacementCheck",
        cancellable = false,
        event_per_listener = true,
        version = 3
    )]
    fn mob_spawn_placement_check(
        dimension: &str,
        kind: &str,
        reason: SpawnReason,
        pos: BlockPos,
        default_result: bool,
        level: LevelRef,
        random: &mut dyn RandomSource,
        result: &mut SpawnCheck,
    );
    /// NeoForge's `MobSpawnEvent.PositionCheck`: may this mob stand at `pos` in `level`?
    /// `Default` runs its own spawn rules and obstruction check. `Succeed` and `Fail`
    /// answer instead, skipping those checks. An ordinary spawner with custom rules
    /// checks only obstruction on `Default`. Natural mobs, chunk-generation packs and
    /// `BaseSpawner` ticks (monster spawner blocks and spawner minecarts) fire this hook;
    /// trial spawners check obstruction directly. `spawner` is `Spawner::Block` for a
    /// monster spawner block, `Spawner::Minecart` for a spawner minecart, and `None`
    /// otherwise. `mob` is a scoped read-only [`UnspawnedMobView`] of the actual new or
    /// loaded candidate. A refusal drops it. Otherwise the same body goes to finalization
    /// and admission. Creating it does not draw from the level's random. Server-side.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "MobSpawnEvent.PositionCheck",
        cancellable = false,
        version = 3
    )]
    fn mob_spawn_position_check(
        dimension: &str,
        kind: &str,
        reason: SpawnReason,
        pos: [f64; 3],
        level: LevelRef,
        spawner: Option<Spawner>,
        mob: UnspawnedMobView<'_>,
        result: &mut SpawnCheck,
    );
}

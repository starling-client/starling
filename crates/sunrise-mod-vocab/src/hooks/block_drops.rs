//! The `block_drops` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
<<<<<<< sync-sunrise-1
    /// NeoForge's `BlockDropsEvent`: a block `state` broke at `pos` and `Block.dropResources` has built the item entities `drops` it will spawn, but nothing is in the world yet. `breaker` is the breaking player's UUID (none for a break the level makes: a piston, water, a neighbour update, `/setblock destroy`) and `tool` the stack they used (none when no tool). `block_entity` is the block's block entity while it is still in the level, to read through the level host (`block_entity_data`): a level-side break builds its drops before the block goes, so a spawner or a container washed away by water names its own; a player's break and an explosion have removed it first and name none. `drops` is the list `Block.popResource` built: each stack with the position, motion and pickup delay its item entity will have, drawn before the event as vanilla draws them, so the level random's order does not depend on whether a mod listens. Edit the list (take out, change or add drops; empty stacks don't spawn) or `experience`, the points `spawnAfterBreak` would award at the block's centre (ores, sculk, spawners; silk touch has already zeroed it, as NeoForge's description says, "the true value, after enchantments"). Set `cancel` and nothing spawns: no drops, no experience, and none of `spawnAfterBreak`'s other effects (an infested block's silverfish); a bee nest's bees are `playerDestroy`'s, not `spawnAfterBreak`'s, and still come out. With the `block_drops` rule off the list and the experience are emptied after the event. It fires on the player's break, a mob's block break that drops loot, explosions and every level-side `Block.dropResources` (a piston breaking a block, water washing one away, a removed support). Player, explosion and mob block loot passes through `loot_drops` first; level-side breaks are not asked `loot_drops`. Explosion events run after the blast has removed its blocks and merged their loot, before any items spawn, with no breaker or tool and zero initial experience. Each block's event owns only its share of a merged stack, at that entity's prepared position and velocity; cancellation cannot remove another block's share. Unchanged shares rejoin the original merged entity, while edited or added entities spawn exactly as returned. Even a block with no loot gets an event (TNT that primes instead does not). Neither registration nor a no-op listener changes the merge or the level random's draws. It does not fire for dispensers or a block's own loot-table drop through another path. It runs inside the level's write, so the level host reads blocks and draws the level's random in vanilla's order and writes nothing. Authoritative servers only. Replaces `block_exp`, whose catalog row named no NeoForge event (`none`) and fired only on the level-side path: the amount is `experience` here. Diverges from upstream: `block_exp` is removed into this hook.
=======
    /// NeoForge's `BlockDropsEvent`: `Block.dropResources` has prepared the item
    /// entities but has not joined them to the level. Fires for player and mob
    /// block breaks using this path, and level-side breaks (pistons, water,
    /// neighbour updates, `/setblock destroy`). Not fired for explosions,
    /// dispensers or other loot-table drops. Use `can_drop_from_explosion` to
    /// decide whether a blasted block drops, and `loot_drops` to edit its stacks
    /// before the blast merges them. Player and mob loot also runs `loot_drops`
    /// first; other level-side breaks do not.
    ///
    /// `breaker` is the breaking player's UUID, if any; `breaker_entity` is the
    /// player or mob that broke the block, if known. `tool` lends the breaking
    /// stack as an [`ItemStackView`]; it is absent if empty or unrepresentable.
    /// Call [`ItemStackView::project`] to copy components on demand.
    /// `block_entity` is the original break subject's
    /// resident handle, never a replacement. If the original was removed or
    /// stored only in the chunk, `block_entity_data` lends its read-only payload,
    /// including `CustomName` but not identity or position. A break of a block
    /// without a block entity has neither field.
    ///
    /// `drops` is a live [`PreparedBlockDrops`] view. Positions and motion have
    /// already been drawn. Inspect scalar fields without projecting components,
    /// edit in place with [`PreparedDropMut`] to keep each item's constructor
    /// RNG, or copy/add/replace entries with owned [`ItemDrop`] captures.
    /// Stack writes validate per edit with [`StackWriteError`], leaving other
    /// entries, experience, and cancellation unchanged if an edit is refused.
    ///
    /// `experience` is the award at the centre after enchantments. When accepted,
    /// item entities join before the experience award. Cancel to suppress the
    /// items, experience and other `spawnAfterBreak` effects (such as silverfish);
    /// bees from `playerDestroy` still come out. With the `block_drops` gamerule
    /// off, positions are still drawn but the initial list is empty and neither
    /// items nor experience spawn. Authoritative server only; uses a read-only
    /// level host.
>>>>>>> sunrise/main
    #[notes = r#"## Block drops

- **`block_drops(level, dimension, pos, state, breaker, breaker_entity, block_entity, block_entity_data, tool, drops, experience, cancel)`** (NeoForge's `BlockDropsEvent`): `Block.dropResources` prepared the item entities but has not joined them to the level. Fires for normal player, mob and level-side breaks, not for explosions, dispensers or other loot-table drops. Use `can_drop_from_explosion` to gate blast drops or `loot_drops` to change blast stacks before merging. `breaker` is the player's UUID; `breaker_entity` is the player or mob whose break runs, if known. The original block entity is lent as a resident `block_entity` or, if removed or stored only in the chunk, as `block_entity_data` (read-only, including `CustomName`, without identity or position). `tool` lends an `ItemStackView`; call `project()` to copy components. `drops` is a live `PreparedBlockDrops` view with positions and motions already drawn. Inspect fields without projecting components, edit in place with `PreparedDropMut` to keep the original RNG, or add/replace/copy with owned `ItemDrop` captures. Stack writes validate individually with `StackWriteError`. Items join before the experience award. Cancel suppresses items, XP and `spawnAfterBreak` effects, not bees released by `playerDestroy`. With the `block_drops` gamerule off, positions are still drawn but the list starts empty and neither items nor XP spawn. Replaces `block_exp` with the event's `experience`."#]
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BlockDropsEvent",
        cancellable = true,
        version = 4,
        target = "sunrise_client::game::native_hooks::block_drops::GameLevel<'_>::native_block_drops"
    )]
    fn block_drops(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        breaker: Option<u128>,
<<<<<<< sync-sunrise-1
        block_entity: Option<BlockEntityRef>,
        tool: Option<&ItemStack>,
        drops: &mut Vec<ItemDrop>,
=======
        breaker_entity: Option<EntityRef>,
        block_entity: Option<BlockEntityRef>,
        block_entity_data: Option<&Compound>,
        tool: Option<ItemStackView<'_>>,
        drops: PreparedBlockDrops<'_>,
>>>>>>> sunrise/main
        experience: &mut u32,
        cancel: &mut bool,
    );
}

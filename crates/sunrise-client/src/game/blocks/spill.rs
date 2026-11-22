//! Block entities removed by the block behaviour (`LevelChunk.setBlockState`
//! running `BlockEntity.preRemoveSideEffects`) and the resources a destroyed
//! block drops (`Block.dropResources`), drawn from the level random at the
//! moment vanilla draws them. The item entities they make are added to the
//! world as soon as the behaviour returns, within the same tick.
use super::api_values::{TryIntoApi, TryIntoEngine};
use super::{Game, GameLevel, Screen};
use mod_api::EntityRef;
use std::collections::HashMap;
use sunrise_client::{
    block_entity_item, block_entity_nbt,
    containers::BlockEntity,
    furnace::FurnaceCooking,
    items::{BlockLootRoll, ItemStack, Items},
};
use sunrise_core::position::BlockPos;
use sunrise_core::random::RandomSource;
use {
    sunrise_core::nbt::{Compound, Tag},
    sunrise_world::{
        level::{LevelEvent, removal, removal::PreRemove},
        registry::BlockStateId,
    },
};

/// Sources retained before removal; the table alone decides which components copy.
#[derive(Default)]
pub(super) struct Pending {
    sources: HashMap<[i32; 3], std::sync::Arc<sunrise_world::loot::copy::BlockSource>>,
    /// A decorated pot's sherds for its dynamic drop, not component-copy policy.
    pub(super) pot: HashMap<[i32; 3], [Option<ItemStack>; 4]>,
}

pub(super) fn source_snapshot(
    mut saved: Tag,
    name: Option<Tag>,
    lock: Option<&Tag>,
) -> sunrise_world::loot::copy::BlockSource {
    if let Tag::Compound(fields) = &mut saved {
        if let Some(name) = name {
            fields.insert("CustomName".into(), name);
        }
        if let Some(lock) = lock {
            fields.insert("lock".into(), lock.clone());
        }
    }
    let nbt = block_entity_nbt::canonical(&saved).unwrap_or(saved);
    let components = block_entity_item::collected_components(&nbt);
    sunrise_world::loot::copy::BlockSource { nbt, components }
}

pub(super) fn world_source(
    world: &sunrise_client::world::World,
    pos: [i32; 3],
    block: &str,
    encode: impl FnOnce((i32, i32, i32), &BlockEntity, &str) -> Tag,
) -> Option<std::sync::Arc<sunrise_world::loot::copy::BlockSource>> {
    let key = (pos[0], pos[1], pos[2]);
    let saved = match world.block_entities.get(&key) {
        Some(entity) => encode(key, entity, block),
        None => saved_entity(world, pos).or_else(|| {
            // Sparse storage can omit an empty block entity, but its type's
            // implicit components still belong to the loot source. Use the
            // factory's entity identity, which need not match the block's id.
            sunrise_client::block_entity_types::type_for(block)
                .or_else(|| sunrise_client::lan::chunks::block_entity_type(block))
                .map(|kind| {
                    Tag::compound([
                        ("id", Tag::String(kind.into())),
                        ("x", Tag::Int(pos[0])),
                        ("y", Tag::Int(pos[1])),
                        ("z", Tag::Int(pos[2])),
                    ])
                })
        })?,
    };
    Some(std::sync::Arc::new(source_snapshot(
        saved,
        world.block_names.get(&key).cloned(),
        world.block_lock(key),
    )))
}

impl Pending {
    pub(super) fn source(
        &self,
        pos: [i32; 3],
    ) -> Option<std::sync::Arc<sunrise_world::loot::copy::BlockSource>> {
        self.sources.get(&pos).cloned()
    }
    pub(super) fn record_from(
        &mut self,
        world: &sunrise_client::world::World,
        pos: [i32; 3],
        saved: Tag,
    ) {
        let key = (pos[0], pos[1], pos[2]);
        let name = world.block_names.get(&key).cloned();
        self.record_source(pos, source_snapshot(saved, name, world.block_lock(key)));
    }
    pub(super) fn record_saved(&mut self, pos: [i32; 3], tag: &sunrise_core::nbt::Tag) {
        self.record_source(pos, source_snapshot(tag.clone(), None, None));
    }
    pub(super) fn record_source(
        &mut self,
        pos: [i32; 3],
        source: sunrise_world::loot::copy::BlockSource,
    ) {
        let source = std::sync::Arc::new(source);
        self.sources.insert(pos, source.clone());
        let tag = &source.nbt;
        if tag.get("id") != Some(&Tag::String("minecraft:decorated_pot".into())) {
            return;
        }
        let sherds = tag.get("sherds");
        let side = |name: &str| match sherds.and_then(|s| s.get(name)) {
            Some(Tag::String(id)) => Some(ItemStack {
                item: std::sync::Arc::from(id.to_string_lossy().as_str()),
                count: 1,
                damage: 0,
                extra: None,
            }),
            Some(t) => sunrise_client::save::item_from_nbt(t),
            None => None,
        };
        self.pot.insert(
            pos,
            [side("back"), side("left"), side("right"), side("front")],
        );
    }
    /// The block entity's dynamic drops: a decorated pot's sherds, left,
    /// back, front then right (`DecoratedPotBlock.getDrops`).
    pub(super) fn dynamic(&self, pos: [i32; 3]) -> Vec<ItemStack> {
        self.pot
            .get(&pos)
            .map_or_else(Vec::new, |[back, left, right, front]| {
                [left, back, front, right]
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect()
            })
    }
    pub(super) fn clear(&mut self, pos: [i32; 3]) {
        self.pot.remove(&pos);
        self.sources.remove(&pos);
    }
}

/// Constructor random retained while drop hooks edit a not-yet-joined item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ItemRandom {
    pub own: (u64, Option<f64>),
    pub uuid: [i32; 4],
    pub bob: f32,
    pub yaw: f32,
    pub velocity: [f64; 3],
    /// The state a drop hook gave the item entity; the default is a fresh one's.
    pub state: DropState,
}

/// Item-entity state a drop hook can set (`ItemDrop`'s `thrower`, `target`, `no_gravity` and
/// `age`). The default is what `new ItemEntity` starts with.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DropState {
    pub(crate) thrower: Option<u128>,
    pub(crate) target: Option<u128>,
    pub(crate) no_gravity: bool,
    pub(crate) age: i32,
}

impl DropState {
    /// The state `drop` asks for.
    pub(crate) fn of(drop: &mod_api::ItemDrop) -> Self {
        Self {
            thrower: drop.thrower,
            target: drop.target,
            no_gravity: drop.no_gravity,
            age: drop.age,
        }
    }
    /// `drop` with this state.
    pub(crate) fn to_api(self, drop: mod_api::ItemDrop) -> mod_api::ItemDrop {
        drop.with_thrower(self.thrower)
            .with_target(self.target)
            .with_no_gravity(self.no_gravity)
            .with_age(self.age)
    }
    /// `item` as a fresh item entity with this state (`setThrower`, `setTarget`,
    /// `setNoGravity` and its age).
    pub(crate) fn apply(
        self,
        item: sunrise_client::entities::ItemEntity,
    ) -> sunrise_client::entities::ItemEntity {
        let uuid = sunrise_client::lan::playerdata::uuid_ints;
        sunrise_client::entities::ItemEntity {
            thrower: self.thrower.map(uuid),
            target: self.target.map(uuid),
            no_gravity: self.no_gravity,
            age: self.age,
            ..item
        }
    }
}

/// The one ItemEntity constructor random sequence, shared by immediate and
/// deferred spawning: UUID, bob offset, yaw, then optional default motion.
pub(super) fn prepare_item_random(velocity: Option<[f64; 3]>) -> ItemRandom {
    let mut own = sunrise_client::random_source::entity();
    let uuid = sunrise_client::save::insecure_uuid(&mut own);
    let bob = own.next_float() * std::f32::consts::PI * 2.0;
    let yaw = own.next_float() * 360.0;
    let velocity = velocity.unwrap_or_else(|| {
        let x = own.next_double() * 0.2 - 0.1;
        [x, 0.2, own.next_double() * 0.2 - 0.1]
    });
    ItemRandom {
        own: (&own).into(),
        uuid,
        bob,
        yaw,
        velocity,
        state: DropState::default(),
    }
}

/// An item entity to add: where, its motion (None: `new ItemEntity`'s own
/// random motion) and its pickup delay.
pub(super) struct Spawn {
    pub(super) random: Option<ItemRandom>,
    pub(super) stack: ItemStack,
    pub(super) pos: [f64; 3],
    pub(super) motion: Option<[f64; 3]>,
    pub(super) delay: u32,
}

/// The block-entity context bound to a block break at the moment the break begins, so a
/// replacement block entity placed at the same position before drops fire (for example by a
/// `player_destroyed_block` listener) never leaks into `block_drops` or `exp_drop`.
#[derive(Clone, Copy)]
pub(in crate::game) enum BlockDropEntity<'a> {
    /// No block entity belongs to this break.
    None,
    /// A block entity that was resident at `pos` with this exact generation when the break
    /// started. Resolves to a live `BlockEntityRef` only while that same generation remains
    /// resident at `pos`.
    Resident(u64, Option<&'a Compound>),
    /// Read-only saved data (including `CustomName`) of a removed or packed block entity.
    /// Never resolves a resident handle at `pos`.
    Saved(&'a Compound),
}

impl<'a> BlockDropEntity<'a> {
    pub(super) fn at(
        world: &sunrise_client::world::World,
        key: (i32, i32, i32),
        data: Option<&'a Compound>,
    ) -> Self {
        if world.block_entities.get(&key).is_some()
            && let Some(generation) = world.block_entities.generation(&key)
        {
            Self::Resident(generation, data)
        } else {
            data.map_or(Self::None, Self::Saved)
        }
    }
}

/// What a broken block spawns (`GameLevel::block_dropped`).
/// Prepared engine item entity; API snapshots exist only while a listener runs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BlockItemDrop {
    pub(super) random: Option<ItemRandom>,
    pub(super) stack: ItemStack,
    pub(super) position: [f64; 3],
    pub(super) velocity: [f64; 3],
    pub(super) pickup_delay: u32,
    /// The item-entity state a drop hook set.
    pub(super) state: DropState,
}

impl BlockItemDrop {
    /// The drop as the mods hear and edit it.
    pub(super) fn from_api(drop: &mod_api::ItemDrop, stack: ItemStack) -> Self {
        Self {
            random: None,
            stack,
            position: drop.position,
            velocity: drop.velocity,
            pickup_delay: drop.pickup_delay,
            state: DropState::of(drop),
        }
    }

    /// The constructor random the item entity joins with, carrying the state a hook set. A drop
    /// with no state set keeps its random as it was (none: drawn when it joins).
    pub(super) fn random_with_state(&self) -> Option<ItemRandom> {
        if self.state == DropState::default() {
            return self.random;
        }
        let mut random = self
            .random
            .unwrap_or_else(|| prepare_item_random(Some(self.velocity)));
        random.state = self.state;
        Some(random)
    }
}

struct EnginePreparedDrops<'a> {
    items: &'a mut Vec<BlockItemDrop>,
    items_registry: &'a Items,
}

impl mod_api::PreparedBlockDropsAccess for EnginePreparedDrops<'_> {
    fn len(&self) -> usize {
        self.items.len()
    }

    fn stack_metadata(
        &self,
        index: usize,
    ) -> Option<(mod_api::ItemKey, std::num::NonZeroU32, Option<u32>)> {
        crate::game::api_values::stack_read_metadata(&self.items.get(index)?.stack).ok()
    }

    fn project_stack(
        &self,
        index: usize,
        metadata: (mod_api::ItemKey, std::num::NonZeroU32, Option<u32>),
    ) -> Option<mod_api::ItemStack> {
        crate::game::api_values::project_stack(&self.items.get(index)?.stack, metadata).ok()
    }

    fn position(&self, index: usize) -> Option<[f64; 3]> {
        self.items.get(index).map(|item| item.position)
    }

    fn velocity(&self, index: usize) -> Option<[f64; 3]> {
        self.items.get(index).map(|item| item.velocity)
    }

    fn pickup_delay(&self, index: usize) -> Option<u32> {
        self.items.get(index).map(|item| item.pickup_delay)
    }

    fn thrower(&self, index: usize) -> Option<Option<u128>> {
        self.items.get(index).map(|item| item.state.thrower)
    }

    fn target(&self, index: usize) -> Option<Option<u128>> {
        self.items.get(index).map(|item| item.state.target)
    }

    fn no_gravity(&self, index: usize) -> Option<bool> {
        self.items.get(index).map(|item| item.state.no_gravity)
    }

    fn age(&self, index: usize) -> Option<i32> {
        self.items.get(index).map(|item| item.state.age)
    }

    fn set_stack(
        &mut self,
        index: usize,
        stack: mod_api::ItemStack,
    ) -> Result<bool, mod_api::StackWriteError> {
        if index >= self.items.len() {
            return Ok(false);
        }
        let converted = stack.try_into_engine(self.items_registry)?;
        self.items[index].stack = converted;
        Ok(true)
    }

    fn set_position(&mut self, index: usize, position: [f64; 3]) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.position = position;
        true
    }

    fn set_velocity(&mut self, index: usize, velocity: [f64; 3]) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.velocity = velocity;
        true
    }

    fn set_pickup_delay(&mut self, index: usize, pickup_delay: u32) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.pickup_delay = pickup_delay;
        true
    }

    fn set_thrower(&mut self, index: usize, thrower: Option<u128>) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.state.thrower = thrower;
        true
    }

    fn set_target(&mut self, index: usize, target: Option<u128>) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.state.target = target;
        true
    }

    fn set_no_gravity(&mut self, index: usize, no_gravity: bool) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.state.no_gravity = no_gravity;
        true
    }

    fn set_age(&mut self, index: usize, age: i32) -> bool {
        let Some(item) = self.items.get_mut(index) else {
            return false;
        };
        item.state.age = age;
        true
    }

    fn push(&mut self, drop: mod_api::ItemDrop) -> Result<(), mod_api::StackWriteError> {
        self.items.push(drop.try_into_engine(self.items_registry)?);
        Ok(())
    }

    fn insert(
        &mut self,
        index: usize,
        drop: mod_api::ItemDrop,
    ) -> Result<bool, mod_api::StackWriteError> {
        if index > self.items.len() {
            return Ok(false);
        }
        self.items
            .insert(index, drop.try_into_engine(self.items_registry)?);
        Ok(true)
    }

    fn replace(
        &mut self,
        index: usize,
        drop: mod_api::ItemDrop,
    ) -> Result<bool, mod_api::StackWriteError> {
        if index >= self.items.len() {
            return Ok(false);
        }
        self.items[index] = drop.try_into_engine(self.items_registry)?;
        Ok(true)
    }

    fn remove(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        self.items.remove(index);
        true
    }

    fn clear(&mut self) {
        self.items.clear();
    }

    fn truncate(&mut self, len: usize) {
        self.items.truncate(len);
    }

    fn swap(&mut self, a: usize, b: usize) -> bool {
        if a >= self.items.len() || b >= self.items.len() {
            return false;
        }
        self.items.swap(a, b);
        true
    }
}

pub(super) struct BlockDrops {
    /// The item entities to add, as the mods left them.
    pub(super) items: Vec<BlockItemDrop>,
    /// The experience points to award at the block's centre.
    pub(super) experience: u32,
    /// A mod canceled the break's drops: `spawnAfterBreak` doesn't run.
    pub(super) canceled: bool,
}

#[derive(Default)]
pub(super) struct Spill {
    pub(super) piston_positions: Vec<super::piston_carry_state::PositionChange>,
    pub(super) spawns: Vec<Spawn>,
    /// Experience orbs (`ExperienceOrb.award`): where and how much, each
    /// and the merge id it drew from the level random.
    pub(super) orbs: Vec<([f64; 3], u32, i32)>,
    /// Block entities the behaviour removed, for the game's own follow-ups
    /// (open screens, playing songs).
    pub(super) removed: Vec<([i32; 3], BlockEntity)>,
    /// Runs of `spawns` a player's use popped (a picked berry bush), for the
    /// mods' `native_loot_drops`.
    pub(super) used: Vec<UsedDrops>,
}

/// `spawns[start..end]` came from `table` at the block `pos` in `state`.
pub(super) struct UsedDrops {
    start: usize,
    end: usize,
    pos: [i32; 3],
    state: BlockStateId,
    table: &'static str,
}

impl Spill {
    pub(super) fn is_empty(&self) -> bool {
        self.piston_positions.is_empty()
            && self.spawns.is_empty()
            && self.orbs.is_empty()
            && self.removed.is_empty()
            && self.used.is_empty()
    }
}

/// Lets `say` change each use's run of `spawns`, newest first so the earlier
/// ranges hold. A run's stacks keep their spots, and a stack `say` adds takes
/// the run's last spot, else the block's top.
fn retell_runs(
    spawns: &mut Vec<Spawn>,
    used: &[UsedDrops],
    mut say: impl FnMut(&UsedDrops, [f64; 3], &mut Vec<ItemStack>),
) {
    for used in used.iter().rev() {
        let run: Vec<Spawn> = spawns.drain(used.start..used.end).collect();
        let mut drops: Vec<ItemStack> = run.iter().map(|s| s.stack.clone()).collect();
        let origin = used.pos.map(|c| f64::from(c) + 0.5);
        say(used, origin, &mut drops);
        let top = [origin[0], origin[1] + 0.5, origin[2]];
        let kept: Vec<Spawn> = drops
            .into_iter()
            .enumerate()
            .map(|(i, stack)| Spawn {
                random: run.get(i).and_then(|s| s.random),
                stack,
                pos: run.get(i).or(run.last()).map_or(top, |s| s.pos),
                motion: run.get(i).and_then(|s| s.motion),
                delay: run.get(i).map_or(10, |s| s.delay),
            })
            .collect();
        spawns.splice(used.start..used.start, kept);
    }
}

/// `Block.popResource`: a spot around the block's centre from the level random.
pub(super) fn pop_position(
    random: &mut impl sunrise_core::random::RandomSource,
    pos: [i32; 3],
) -> [f64; 3] {
    let mut offset = || -0.25 + random.next_double() * 0.5;
    // EntityTypes.ITEM is 0.25 high.
    let half_height = f64::from(0.25f32) / 2.0;
    [
        f64::from(pos[0]) + 0.5 + offset(),
        f64::from(pos[1]) + 0.5 + offset() - half_height,
        f64::from(pos[2]) + 0.5 + offset(),
    ]
}

/// `Block.popResource` draws the level offsets even when the rule prevents
/// constructing (or capturing) an item entity.
pub(super) fn admitted_pop_position(
    random: &mut impl RandomSource,
    pos: [i32; 3],
    block_drops: bool,
) -> Option<[f64; 3]> {
    let at = pop_position(random, pos);
    block_drops.then_some(at)
}

/// `Block.popResourceFromFace`'s position and motion: an item on the clicked
/// face, moved off that face and jittered across the other two axes.
pub(super) fn pop_position_from_face(
    random: &mut impl sunrise_core::random::RandomSource,
    pos: [i32; 3],
    face: [i32; 3],
) -> ([f64; 3], [f64; 3]) {
    fn jitter<R: sunrise_core::random::RandomSource>(random: &mut R, min: f64, max: f64) -> f64 {
        min + random.next_double() * (max - min)
    }
    // EntityTypes.ITEM is 0.25 wide and 0.25 high.
    let (half_width, half_height) = (0.125, 0.125);
    let x = f64::from(pos[0])
        + 0.5
        + if face[0] == 0 {
            jitter(random, -0.25, 0.25)
        } else {
            f64::from(face[0]) * (0.5 + half_width)
        };
    let y = f64::from(pos[1])
        + 0.5
        + if face[1] == 0 {
            jitter(random, -0.25, 0.25)
        } else {
            f64::from(face[1]) * (0.5 + half_height)
        }
        - half_height;
    let z = f64::from(pos[2])
        + 0.5
        + if face[2] == 0 {
            jitter(random, -0.25, 0.25)
        } else {
            f64::from(face[2]) * (0.5 + half_width)
        };
    let dx = if face[0] == 0 {
        jitter(random, -0.1, 0.1)
    } else {
        f64::from(face[0]) * 0.1
    };
    let dy = if face[1] == 0 {
        jitter(random, 0.0, 0.1)
    } else {
        f64::from(face[1]) * 0.1 + 0.1
    };
    let dz = if face[2] == 0 {
        jitter(random, -0.1, 0.1)
    } else {
        f64::from(face[2]) * 0.1
    };
    ([x, y, z], [dx, dy, dz])
}

/// The block entity at `pos` Sunrise keeps as it was loaded, if any
/// (campfires, decorated pots, chiseled bookshelves, shelves).
pub(super) fn saved_entity(
    world: &sunrise_client::world::World,
    pos: [i32; 3],
) -> Option<sunrise_core::nbt::Tag> {
    use sunrise_core::nbt::Tag;
    let saved = world
        .chunks
        .get(&(pos[0] >> 4, pos[2] >> 4))?
        .saved
        .as_ref()?;
    saved
        .block_entities
        .iter()
        .find(|t| {
            t.get("x") == Some(&Tag::Int(pos[0]))
                && t.get("y") == Some(&Tag::Int(pos[1]))
                && t.get("z") == Some(&Tag::Int(pos[2]))
        })
        .cloned()
}

/// The read-only payload of an entity kept in packed chunk data, not its loader metadata.
pub(super) fn packed_block_entity_payload(tag: Tag) -> Option<Compound> {
    let Tag::Compound(mut data) = tag else {
        return None;
    };
    data.remove(&"keepPacked".into());
    Some(super::native_block_entity_refs::block_entity_payload(data))
}

/// Keep `tag` as the block entity at its position Sunrise does not model,
/// replacing the one there.
pub(super) fn put_saved_entity(
    world: &mut sunrise_client::world::World,
    pos: [i32; 3],
    tag: sunrise_core::nbt::Tag,
) {
    set_saved_entity(world, pos, Some(tag));
}

/// Restore a saved-only block entity, or remove the tag created by a tentative write.
pub(super) fn set_saved_entity(
    world: &mut sunrise_client::world::World,
    pos: [i32; 3],
    tag: Option<sunrise_core::nbt::Tag>,
) {
    use sunrise_core::nbt::Tag;
    let Some(chunk) = world.chunks.get_mut(&(pos[0] >> 4, pos[2] >> 4)) else {
        return;
    };
    let chunk = std::sync::Arc::make_mut(chunk);
    let saved = match tag {
        Some(_) => chunk.saved.get_or_insert_default(),
        None => match &mut chunk.saved {
            Some(saved) => saved,
            None => return,
        },
    };
    let rows = &mut std::sync::Arc::make_mut(saved).block_entities;
    let at = rows.iter().position(|t| {
        t.get("x") == Some(&Tag::Int(pos[0]))
            && t.get("y") == Some(&Tag::Int(pos[1]))
            && t.get("z") == Some(&Tag::Int(pos[2]))
    });
    match (at, tag) {
        (Some(index), Some(tag)) => rows[index] = tag,
        (Some(index), None) => {
            rows.remove(index);
        }
        (None, Some(tag)) => rows.push(tag),
        (None, None) => {}
    }
}

/// How to finish the removal after its pre-removal effects have run.
pub(super) enum RemovingBlockEntity {
    Saved,
    Resident {
        generation: Option<u64>,
        kind: PreRemove,
        neighbor_updates: u8,
    },
    OwnTick,
}

#[derive(Default)]
pub(in crate::game) struct RemovalOutputs {
    pub keep_contents: bool,
    pub drops: Vec<Option<ItemStack>>,
    pub spawns: Vec<(ItemStack, [f64; 3])>,
    pub experience: Vec<(u32, [f64; 3])>,
}

impl GameLevel<'_> {
    fn pop_resource(&mut self, pos: [i32; 3], stack: ItemStack) {
        if let Some(at) = admitted_pop_position(&mut self.world.random, pos, self.rules.block_drops)
        {
            self.spill.spawns.push(Spawn {
                random: None,
                stack,
                pos: at,
                motion: None,
                delay: 10,
            });
        }
    }

    /// `Block.popResource(level, pos, stack)` with the broken block's copied components.
    /// `Block.popResource(level, pos, new ItemStack(block))`.
    pub(super) fn pop_block_item(&mut self, pos: BlockPos, state: BlockStateId) {
        let item = self.facts.get(state).item.clone();
        if &*item != "minecraft:air" {
            self.pop_resource([pos.x, pos.y, pos.z], ItemStack::new(&item, 1));
        }
    }
    /// An item `atLowerCornerWithOffset(pos, 0.5, 1.01, 0.5).offsetRandomXZ(random,
    /// 0.7)` with the default pickup delay (`ComposterBlock.extractProduce`).
    pub(super) fn pop_above(&mut self, pos: BlockPos, item: std::sync::Arc<str>) {
        let at = removal::jukebox_pop_position(&mut self.world.random, pos);
        self.spill.spawns.push(Spawn {
            random: None,
            stack: ItemStack::new(&item, 1),
            pos: at,
            motion: None,
            delay: 10,
        });
    }
    /// `Block.dropFromBlockInteractLootTable` with `Block.popResource` of
    /// every stack.
    pub(super) fn drop_interact_loot(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        table: &'static str,
    ) {
        let loot = self.loot.clone();
        let matcher = |predicate: &serde_json::Value, entity: &sunrise_world::loot::LootEntity| {
            loot.as_ref()
                .is_some_and(|data| data.entity_matches(predicate, Some(entity)))
        };
        let interacting = self.interacting.as_ref().map(|entity| {
            (
                entity,
                &matcher as &dyn Fn(&serde_json::Value, &sunrise_world::loot::LootEntity) -> bool,
            )
        });
        // `LocationCheck` reads from ORIGIN, the block's centre, through the
        // advancement path's `LocationPredicate` over this level's own state.
        // The roll draws from a copy of the level random, which the location
        // reads of the world leave no hand on.
        let origin = [pos.x, pos.y, pos.z];
        let level = super::location_world::LevelLocation {
            world: &*self.world,
            registry: self.registry,
            facts: self.facts,
            biomes: self.biomes,
            items: self.items,
            collision: self.collision,
            hoppers: &*self.positions.hoppers,
            raining: self.raining,
            thundering: self.thundering,
            refs: self.location,
        };
        let location = |condition: &serde_json::Value| {
            let centre = origin.map(|c| f64::from(c) + 0.5);
            let at = sunrise_client::advancements::location_check_position(
                condition,
                Some(origin),
                centre,
            );
            // `LocationPredicate.matches`: nothing matches in an unloaded chunk.
            Some(
                level
                    .world
                    .loaded(at[0].floor() as i32, at[2].floor() as i32)
                    && sunrise_client::advancements::location(&condition["predicate"], at, &level),
            )
        };
        let mut random = self.world.random.clone();
        let drops = self.items.interact_loot(
            table,
            self.registry,
            state,
            interacting,
            Some(&location),
            &mut random,
            self.rand_sequences,
            self.seed,
        );
        self.world.random = random;
        let start = self.spill.spawns.len();
        for stack in drops {
            if &*stack.item != "minecraft:air" && stack.count > 0 {
                self.pop_resource([pos.x, pos.y, pos.z], stack);
            }
        }
        self.spill.used.push(UsedDrops {
            start,
            end: self.spill.spawns.len(),
            pos: [pos.x, pos.y, pos.z],
            state,
            table,
        });
    }
    /// `Block.dropResources(state, level, pos, blockEntity)`: the block's loot
    /// (from its random sequence, else the level random), with what its block
    /// entity copies into it, popped at once. While a mod's machine breaks
    /// the block (`native_machine_destroy_block`), the loot is rolled with its
    /// `drop_tool` (`dropResources(.., entity, tool)`) and, given `drop_into`,
    /// kept there instead of popped.
    pub(super) fn drop_resources_now(&mut self, pos: BlockPos, state: BlockStateId) {
        let key = (pos.x, pos.y, pos.z);
        let at = [pos.x, pos.y, pos.z];
        self.unpack_loot(super::container_loot::LootTarget::Block(key), None);
        let packed_data = if let Some(entity) = self.world.block_entities.get(&key) {
            let saved = self.encode_live_block_entity(key, entity, &self.facts.get(state).name);
            self.pending.record_from(self.world, at, saved);
            None
        } else if let Some(tag) = saved_entity(self.world, at) {
            self.pending.record_saved(at, &tag);
            if self.native_authoritative
                && mod_api::__private::listened(mod_api::HookId::BlockDrops)
            {
                packed_block_entity_payload(tag)
            } else {
                None
            }
        } else {
            None
        };
        let block_entity = BlockDropEntity::at(self.world, key, packed_data.as_ref());
        let dynamic = self.pending.dynamic(at);
        let machine = self
            .drop_tool
            .as_ref()
            .is_some_and(|(target, _)| *target == key);
        let tool = self
            .drop_tool
            .as_ref()
            .filter(|_| machine)
            .and_then(|(_, tool)| tool.clone());
        let roll = sunrise_client::items::BlockLootRoll {
            held: tool.as_ref(),
            dynamic: &dynamic,
            breaker: self.drop_breaker_at == Some(pos),
            ..Default::default()
        };
        let entity = self
            .drop_entity
            .filter(|_| self.drop_breaker_at == Some(pos));
        let drops = self.roll_block_loot(pos, state, entity, roll, None);
        let dropped =
            self.block_dropped(pos, state, None, tool.as_ref(), block_entity, drops, None);
        for item in dropped.items {
            if let Some(kept) = self.drop_into.as_mut().filter(|_| machine) {
                kept.push(item.stack);
            } else {
                self.spill.spawns.push(Spawn {
                    random: item.random_with_state(),
                    stack: item.stack,
                    pos: item.position,
                    motion: Some(item.velocity),
                    delay: item.pickup_delay,
                });
            }
        }
        self.pending.clear(at);
        if dropped.experience > 0 {
            let centre = [
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 0.5,
                f64::from(pos.z) + 0.5,
            ];
            self.award_orbs(centre, dropped.experience);
        }
    }
    /// `Block.getDrops(state, level, pos, blockEntity, entity, tool)` for the block still
    /// standing: `drop_resources_now`'s roll, with what the block entity copies into the drops,
    /// drawn from the same random, but nothing popped, no `block_drops` event and no
    /// experience. A scratch record keeps a break in progress at `pos` as it was.
    pub(super) fn block_drops_of(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        tool: Option<&ItemStack>,
        entity: Option<EntityRef>,
    ) -> Vec<ItemStack> {
        let key = (pos.x, pos.y, pos.z);
        let at = [pos.x, pos.y, pos.z];
        let mut copied = Pending::default();
        if let Some(block_entity) = self.world.block_entities.get(&key) {
            let saved =
                self.encode_live_block_entity(key, block_entity, &self.facts.get(state).name);
            copied.record_from(self.world, at, saved);
        } else if let Some(tag) = saved_entity(self.world, at) {
            copied.record_saved(at, &tag);
        }
        let dynamic = copied.dynamic(at);
        let roll = sunrise_client::items::BlockLootRoll {
            held: tool,
            dynamic: &dynamic,
            breaker: entity.is_some(),
            ..Default::default()
        };
        let source = copied.source(at);
        let mut drops = self.roll_block_loot(pos, state, entity, roll, source.as_deref());
        drops.retain(|stack| &*stack.item != "minecraft:air" && stack.count > 0);
        drops
    }
    /// What `Block.dropResources` does with the loot it rolled for the block
    /// `state` at `pos`: `Block.popResource` builds an item entity for each
    /// stack (its position drawn from the level random, its motion from the
    /// item's own), then `spawnAfterBreak` samples the block's experience,
    /// in that order. The mods then see the list (`block_drops`, NeoForge's
    /// `BlockDropsEvent`). Nothing is spawned here: the caller adds the
    /// entities and orbs, and a canceled break leaves both empty. The random
    /// draws happen with or without a listener, in vanilla's order.
    pub(super) fn block_dropped(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        breaker: Option<u128>,
        tool: Option<&ItemStack>,
        block_entity: BlockDropEntity<'_>,
        stacks: Vec<ItemStack>,
        exp_to_drop: Option<u32>,
    ) -> BlockDrops {
        let mut items = stacks
            .into_iter()
            .filter(|stack| &*stack.item != "minecraft:air" && stack.count > 0)
            .filter_map(|stack| {
                let position = admitted_pop_position(
                    &mut self.world.random,
                    [pos.x, pos.y, pos.z],
                    self.rules.block_drops,
                )?;
                let random = prepare_item_random(None);
                Some(BlockItemDrop {
                    random: Some(random),
                    stack,
                    position,
                    velocity: random.velocity,
                    pickup_delay: 10,
                    state: DropState::default(),
                })
            })
            .collect();
        // `tryDropExperience` samples before the silk touch effect zeroes the
        // part/vanilla starting amount before the global break_block event; a
        // spawner ignores the tool. An explicit global override is applied as given.
        let name = self.facts.get(state).name.clone();
        let silk = tool.is_some_and(|tool| tool.enchantment_level("minecraft:silk_touch") > 0);
        let experience = exp_to_drop
            .or_else(|| self.part_break_experience(pos, state, breaker, tool, block_entity))
            .unwrap_or_else(|| {
                removal::break_experience(&mut self.world.random, &name)
                    .filter(|_| !silk || &*name == "minecraft:spawner")
                    .unwrap_or(0)
            });
        let mut experience = if exp_to_drop.is_none() && silk && &*name != "minecraft:spawner" {
            0
        } else {
            experience
        };
        let mut canceled = false;
        if self.native_authoritative
            && mod_api::__private::listened(mod_api::HookId::BlockDrops)
            && let Ok(api_state) = state.try_into_api()
        {
            let tool_metadata = tool.and_then(|stack| {
                if stack.count == 0 || &*stack.item == "minecraft:air" {
                    return None;
                }
                match crate::game::api_values::stack_read_metadata(stack) {
                    Ok(metadata) => Some(metadata),
                    Err(error) => {
                        log::warn!(
                            "block_drops at {pos:?}: omitting unrepresentable tool '{}' (count {}): {error:?}",
                            stack.item,
                            stack.count,
                        );
                        None
                    }
                }
            });
            let (block_entity_ref, breaker_entity) =
                self.drop_event_refs(pos, breaker, block_entity);
            let block_entity_data = match block_entity {
                BlockDropEntity::Saved(data) => Some(data),
                BlockDropEntity::Resident(_, data) if block_entity_ref.is_none() => data,
                BlockDropEntity::None | BlockDropEntity::Resident(..) => None,
            };
            let project_tool = |metadata| {
                tool.and_then(|stack| crate::game::api_values::project_stack(stack, metadata).ok())
            };
            let tool_view = tool_metadata
                .map(|metadata| mod_api::ItemStackView::resident(metadata, &project_tool));
            let mut drops_access = EnginePreparedDrops {
                items: &mut items,
                items_registry: self.items,
            };
            self.native_block_drops(mod_api::__private::BlockDropsCall {
                dimension: self.dimension,
                pos,
                state: api_state,
                breaker,
                breaker_entity,
                block_entity: block_entity_ref,
                block_entity_data,
                tool: tool_view,
                drops: mod_api::PreparedBlockDrops::new(&mut drops_access),
                experience: &mut experience,
                cancel: &mut canceled,
            });
        }
        if canceled || !self.rules.block_drops {
            items.clear();
            experience = 0;
        }
        BlockDrops {
            items,
            experience,
            canceled,
        }
    }
    /// `ExperienceOrb.award`: orbs of the largest sizes, each drawing the id
    /// `tryMergeToExisting` compares from the level random.
    pub(super) fn award_orbs(&mut self, at: [f64; 3], mut amount: u32) {
        while amount > 0 {
            let value = removal::orb_value(amount);
            amount -= value;
            let merge_id = self.world.random.next_int_bound(40).unwrap_or(0);
            self.spill.orbs.push((at, value, merge_id));
        }
    }
    /// `BlockEntity.preRemoveSideEffects` and `removeBlockEntity` for the block
    /// entity of `old` at `pos`. A block entity Sunrise never created reads as
    /// vanilla's fresh one: an empty container still draws a spot per slot.
    pub(crate) fn remove_entity(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        side_effects: bool,
    ) -> u8 {
        let removal = self.pre_remove_entity(pos, old, side_effects);
        self.finish_remove_entity(pos, old, removal)
    }

    pub(super) fn pre_remove_entity(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        side_effects: bool,
    ) -> RemovingBlockEntity {
        let key = (pos.x, pos.y, pos.z);
        self.unpack_loot(super::container_loot::LootTarget::Block(key), None);
        // `Containers.dropItemStack` splits the original stack before adding the item.
        // A saved-only row has no live slots to drain. Take its backing data here, so a
        // join listener that replaces this cell again cannot decode the same stack twice.
        if let Some(tag) = self.take_saved_entity(pos) {
            self.positions.hoppers.remove(&key);
            if side_effects {
                self.drop_saved_contents(pos, old, tag);
            }
            return RemovingBlockEntity::Saved;
        }
        // Sculk blocks and creaking hearts notice their removal on their own tick.
        if matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Sculk(_) | BlockEntity::CreakingHeart(_))
        ) {
            return RemovingBlockEntity::OwnTick;
        }
        let generation = self.world.block_entities.generation(&key);
        // A mod's block entity answers its parts in place (its slot, so its
        // attachments, stay until `put`), as vanilla's override runs on `this`.
        let mut outputs = RemovalOutputs::default();
        if side_effects
            && matches!(
                self.world.block_entities.get(&key),
                Some(BlockEntity::Mod(_))
            )
            && let Some((lease, BlockEntity::Mod(entity))) = self.world.block_entities.take(&key)
        {
            outputs = self.api_pre_remove_side_effects(pos, old, &entity);
            self.world
                .block_entities
                .put(lease, BlockEntity::Mod(entity));
        }
        let entity = self.world.block_entities.get(&key);
        let name = self.facts.get(old).name.clone();
        // BaseContainerBlockEntity.preRemoveSideEffects: Containers.dropContents
        // visits every declared slot, even if its empty storage hasn't been materialized.
        let kind = if let Some(BlockEntity::Mod(m)) = entity
            && let Some(container) = self.world.block_entities.mod_container(old.0)
        {
            PreRemove::Contents {
                slots: container.slots.max(m.items.len()),
                furnace: false,
            }
        } else {
            removal::pre_remove(&name)
        };
        let mut neighbour_updates = 0;
        if side_effects {
            match kind {
                PreRemove::Contents { slots, furnace } => {
                    for slot in 0..if outputs.keep_contents { 0 } else { slots } {
                        let stack = self
                            .world
                            .block_entities
                            .get_mut(&key)
                            .and_then(|row| row.slots_mut().get_mut(slot))
                            .and_then(Option::take);
                        self.drop_item_stack(pos, stack);
                    }
                    // AbstractFurnaceBlockEntity.preRemoveSideEffects: then the
                    // experience of what it made, at the block's centre.
                    if furnace
                        && let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get(&key)
                    {
                        let at = [
                            f64::from(pos.x) + 0.5,
                            f64::from(pos.y) + 0.5,
                            f64::from(pos.z) + 0.5,
                        ];
                        for (count, experience) in f.recipe_experience(self.recipes) {
                            let xp = sunrise_client::containers::create_experience(
                                count,
                                experience,
                                &mut self.world.random,
                            );
                            self.award_orbs(at, xp);
                        }
                    }
                }
                PreRemove::Campfire => {
                    let items = match self.world.block_entities.get_mut(&key) {
                        Some(BlockEntity::Campfire(c)) => std::mem::take(&mut c.items),
                        _ => Default::default(),
                    };
                    for stack in items {
                        self.drop_item_stack(pos, stack);
                    }
                }
                PreRemove::Jukebox => {
                    // popOutTheItem: removeTheItem stops a playing song (its
                    // events and a neighbour update), the record pops, and
                    // onSongChanged updates the neighbours again.
                    if let Some(BlockEntity::Jukebox(j)) = self.world.block_entities.get_mut(&key)
                        && let Some(record) = j.items[0].take()
                    {
                        if j.playing {
                            self.jukebox_stopped(pos, old);
                            neighbour_updates += 1;
                        }
                        let at = removal::jukebox_pop_position(&mut self.world.random, pos);
                        self.spawn_removal_item(record, at);
                        neighbour_updates += 1;
                    }
                }
                PreRemove::Lectern => {
                    let props = self.registry.state(old).map(|s| &s.properties);
                    let has_book = props
                        .and_then(|p| p.get("has_book"))
                        .is_some_and(|v| v == "true");
                    let step = match props.and_then(|p| p.get("facing")).map(String::as_str) {
                        Some("north") => [0, -1],
                        Some("south") => [0, 1],
                        Some("west") => [-1, 0],
                        _ => [1, 0],
                    };
                    if has_book
                        && let Some(BlockEntity::Lectern(l)) =
                            self.world.block_entities.get_mut(&key)
                        && let Some(book) = l.book.take()
                    {
                        self.spawn_removal_item(book, removal::lectern_book_position(pos, step));
                    }
                }
                PreRemove::ShulkerBox
                | PreRemove::SculkShrieker
                | PreRemove::CreakingHeart
                | PreRemove::Beacon
                | PreRemove::Nothing => {}
            }
            for stack in outputs.drops {
                self.drop_item_stack(pos, stack);
            }
            for (stack, at) in outputs.spawns {
                self.spawn_removal_item(stack, at);
            }
            for (amount, at) in outputs.experience {
                self.award_orbs(at, amount);
            }
        }
        RemovingBlockEntity::Resident {
            generation,
            kind,
            neighbor_updates: neighbour_updates,
        }
    }

    pub(super) fn finish_remove_entity(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        removal: RemovingBlockEntity,
    ) -> u8 {
        let key = (pos.x, pos.y, pos.z);
        // A drop's join listener can replace the row, even with one for the same block state.
        // Don't unregister that row or clear its metadata; only finish the pre-removal updates.
        if let RemovingBlockEntity::Resident {
            generation,
            neighbor_updates,
            ..
        } = removal
            && self.world.block_entities.generation(&key) != generation
        {
            return neighbor_updates;
        }
        // Headless levels cannot rely on a renderer to discard a replaced gateway.
        if &*self.facts.get(old).name == "minecraft:end_gateway" {
            self.positions.gateways.remove(&[pos.x, pos.y, pos.z]);
        }
        let (kind, neighbour_updates) = match removal {
            RemovingBlockEntity::OwnTick | RemovingBlockEntity::Saved => return 0,
            RemovingBlockEntity::Resident {
                kind,
                neighbor_updates,
                ..
            } => (kind, neighbor_updates),
        };
        let entity = self.world.block_entities.remove(&key);
        self.positions.hoppers.remove(&key);
        self.world.block_names.remove(&key);
        self.world.forget_block_entity_source(key);
        // BlockEntity.setRemoved: a jukebox always reports its song stopping,
        // a beacon plays its deactivation.
        match kind {
            PreRemove::Jukebox => self.jukebox_stopped(pos, old),
            PreRemove::Beacon => self.events.push(LevelEvent::Sound {
                pos,
                sound: "minecraft:block.beacon.deactivate".into(),
                source: None,
                volume: 1.0,
                pitch: 1.0,
            }),
            _ => {}
        }
        if let Some(entity) = entity {
            self.spill.removed.push(([pos.x, pos.y, pos.z], entity));
        }
        neighbour_updates
    }
    /// ItemEntity(level, x, y, z, stack), followed by setDefaultPickUpDelay.
    fn spawn_removal_item(&mut self, stack: ItemStack, pos: [f64; 3]) {
        self.spill.spawns.push(Spawn {
            random: None,
            stack,
            pos,
            motion: None,
            delay: 10,
        });
    }
    /// The jukebox's `JUKEBOX_STOP_PLAY` game event and level event 1011.
    fn jukebox_stopped(&mut self, pos: BlockPos, state: BlockStateId) {
        self.events.push(LevelEvent::GameEvent {
            pos,
            event: "minecraft:jukebox_stop_play",
            state: Some(state),
        });
        self.events.push(LevelEvent::LevelEvent {
            id: 1011,
            pos,
            data: 0,
            source: None,
        });
    }
    fn take_saved_entity(&mut self, pos: BlockPos) -> Option<Tag> {
        let chunk = self.world.chunks.get_mut(&(pos.x >> 4, pos.z >> 4))?;
        let index = chunk
            .saved
            .as_ref()?
            .block_entities
            .iter()
            .position(|tag| {
                tag.get("x") == Some(&Tag::Int(pos.x))
                    && tag.get("y") == Some(&Tag::Int(pos.y))
                    && tag.get("z") == Some(&Tag::Int(pos.z))
            })?;
        let saved = std::sync::Arc::make_mut(chunk).saved.as_mut()?;
        Some(std::sync::Arc::make_mut(saved).block_entities.remove(index))
    }

    /// Spill the contents of a block entity still backed by its saved compound.
    fn drop_saved_contents(
        &mut self,
        pos: BlockPos,
        old: BlockStateId,
        tag: sunrise_core::nbt::Tag,
    ) {
        let name = self.facts.get(old).name.clone();
        let items = match removal::pre_remove(&name) {
            PreRemove::Contents { slots, .. } if &*name == "minecraft:decorated_pot" => {
                // Containers.dropContents reads getTheItem, which unpacks the loot table.
                let mut tag = tag;
                super::decorated_pot::unpack_loot(
                    &mut tag,
                    self.loot.as_deref(),
                    &mut self.world.random,
                    self.rand_sequences,
                    self.seed,
                    self.items,
                    sunrise_world::loot::ContainerLootContext::default(),
                );
                let mut items = vec![None; slots];
                items[0] = tag
                    .get("item")
                    .and_then(sunrise_client::save::item_from_nbt);
                items
            }
            PreRemove::Contents { slots, .. } => {
                sunrise_client::save::decode_items(tag.get("Items"), slots)
            }
            PreRemove::Campfire => sunrise_client::save::decode_items(tag.get("Items"), 4),
            _ => return,
        };
        for stack in items {
            self.drop_item_stack(pos, stack);
        }
    }
    /// `Containers.dropItemStack` of one slot (an empty one still draws its spot).
    fn drop_item_stack(&mut self, pos: BlockPos, stack: Option<ItemStack>) {
        let count = stack.as_ref().map_or(0, |s| s.count);
        for piece in removal::drop_item_stack(&mut self.world.random, pos, count) {
            let Some(stack) = &stack else { break };
            self.spill.spawns.push(Spawn {
                random: None,
                stack: ItemStack {
                    count: piece.count,
                    ..stack.clone()
                },
                pos: piece.pos,
                motion: Some(piece.motion),
                delay: 0,
            });
        }
    }
}

impl Game<'_> {
    /// [`GameLevel::roll_block_loot`] for a roll outside a level write:
    /// `entity` is `THIS_ENTITY`'s id. With no listener it builds no level.
    pub(in crate::game) fn roll_block_loot(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        entity: Option<u32>,
        roll: BlockLootRoll<'_>,
    ) -> Vec<ItemStack> {
        let source = self.pending.source(pos).or_else(|| {
            self.pack.assets.registry.state(state).and_then(|block| {
                world_source(&self.world, pos, &block.name, |key, entity, block| {
                    self.encode_live_block_entity(key, entity, block, world_items!(self))
                })
            })
        });
        let storage = self.score.storage.snapshot();
        sunrise_world::loot::copy::with_storage(storage, || {
            sunrise_world::loot::copy::with_block_source(source.as_deref(), || {
                if mod_api::__private::listened(mod_api::HookId::EnchantedBlockLoot) {
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    let entity = self.loot_entity_ref(entity);
                    return self.with_level(|_, level, _| {
                        level.roll_block_loot(at, state, entity, roll, source.as_deref())
                    });
                }
                world_items!(self).block_drops(
                    self.pack.assets.registry,
                    state,
                    roll,
                    &mut self.world.random,
                    Some((&mut self.rand_sequences, self.level.seed)),
                )
            })
        })
    }
    /// The handle of `LootContextParams.THIS_ENTITY`, by its id.
    pub(in crate::game) fn loot_entity_ref(&self, id: Option<u32>) -> Option<EntityRef> {
        let epoch = self.native_lifecycle.epoch()?;
        Some(EntityRef::new(epoch, id.filter(|id| *id != 0)?))
    }
    /// Apply the shared block-drop event's item entities and experience unchanged.
    pub(super) fn spawn_block_drops(&mut self, pos: [i32; 3], dropped: BlockDrops) {
        for item in dropped.items {
            self.add_item_drop(item);
        }
        if dropped.experience > 0 {
            self.spawn_orbs(pos.map(|v| f64::from(v) + 0.5), dropped.experience);
        }
    }
    /// Adds a drop's item entity, with the state a drop hook set.
    pub(super) fn add_item_drop(&mut self, drop: BlockItemDrop) {
        let random = drop.random_with_state();
        let velocity = Some(drop.velocity);
        self.add_item_entity_prepared(
            drop.stack,
            drop.position,
            velocity,
            drop.pickup_delay,
            random,
        );
    }
    pub(super) fn publish_removal_spawns(
        &mut self,
        spawns: Vec<Spawn>,
        orbs: Vec<([f64; 3], u32, i32)>,
    ) {
        let _save_lease = sunrise_client::join_seq::visibility::PassLease::enter();
        for spawn in spawns {
            self.push_item_entity(
                spawn.stack,
                spawn.pos,
                spawn.motion,
                spawn.delay,
                spawn.random,
            );
            self.join_removal_item(self.next_entity, spawn.pos);
        }
        for (at, value, merge_id) in orbs {
            self.award_orb(at, value, merge_id);
        }
    }

    /// Add the item entities the block behaviour spawned and follow up on the
    /// block entities it removed.
    pub(super) fn flush_spill(&mut self) {
        if self.spill.is_empty() {
            return;
        }
        let _save_lease = sunrise_client::join_seq::visibility::PassLease::enter();
        let mut spill = std::mem::take(&mut self.spill);
        self.finish_piston_position_changes(std::mem::take(&mut spill.piston_positions));
        let used = std::mem::take(&mut spill.used);
        let tool = self.held().cloned();
        let player = Some(self.native_acting_id());
        retell_runs(&mut spill.spawns, &used, |used, origin, drops| {
            let block = Some((used.pos, used.state));
            self.native_use_drops(
                used.table,
                origin,
                block,
                None,
                player,
                tool.as_ref(),
                drops,
            );
        });
        for s in spill.spawns {
            self.add_item_entity_prepared(s.stack, s.pos, s.motion, s.delay, s.random);
        }
        for (at, value, merge_id) in spill.orbs {
            self.award_orb(at, value, merge_id);
        }
        for (pos, entity) in spill.removed {
            let key = (pos[0], pos[1], pos[2]);
            if matches!(entity, BlockEntity::Jukebox(_)) {
                self.jukebox_removed(pos);
            }
            if self.container == Some(key) {
                self.container_opener_event(key, false);
                self.container = None;
                self.set_screen(Screen::Playing);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RemovalMod(u8);
    struct RemovalPart(u8, mod_api::ItemKey);
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for RemovalPart {}
    #[mod_api::sunrise_part]
    impl mod_api::BlockEntityPart for RemovalPart {
        fn pre_remove_side_effects(
            &self,
            _: &mut dyn mod_api::LevelRandomHost,
            event: &mut mod_api::PreRemoveSideEffectsEvent<'_>,
        ) {
            match self.0 {
                1 => event.keep_contents = true,
                2 => {
                    let key = self.1;
                    event.spawns.push((
                        mod_api::ItemStack::new(key, 1).unwrap(),
                        [1.25, 65.75, -2.5],
                    ));
                }
                3 => event.experience.push((10, [1.25, 65.75, -2.5])),
                4 => {
                    event.drops.push(None);
                    let key = self.1;
                    event
                        .drops
                        .push(Some(mod_api::ItemStack::new(key, 1).unwrap()));
                }
                _ => {}
            }
        }
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for RemovalMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let kind = r.block_entity_type("machine").blocks(&[]).build();
            let diamond = r.item_key("minecraft:diamond");
            r.block_entity_part(&kind, RemovalPart(self.0, diamond))
                .unwrap();
        }
    }
    fn removal_outputs(
        mode: u8,
        check: impl FnOnce(&mut GameLevel<'_>, &mut sunrise_core::random::LevelRandom),
    ) {
        use std::sync::Arc;
        use sunrise_client::containers::{ModBlockEntity, ModBlockEntityFields, ModContainer};
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "removal",
            Box::new(RemovalMod(mode)),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let mut entity =
                    ModBlockEntity::new("removal:machine".into(), ModBlockEntityFields::default());
                if mode == 1 {
                    entity.items = vec![Some(ItemStack::new("minecraft:diamond", 1))];
                    game.world.block_entities.set_mod_containers(
                        [(
                            state.0,
                            Arc::new(ModContainer {
                                slots: 1,
                                ..Default::default()
                            }),
                        )]
                        .into_iter()
                        .collect(),
                    );
                }
                game.world
                    .block_entities
                    .insert((1, 64, 2), BlockEntity::Mod(Box::new(entity)));
                game.with_level(|_, level, _| {
                    let mut reference = level.world.random.clone();
                    level.remove_entity(BlockPos::new(1, 64, 2), state, true);
                    check(level, &mut reference);
                });
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_keep_contents_suppresses_slot_scatter() {
        removal_outputs(1, |level, reference| {
            assert!(level.spill.spawns.is_empty());
            assert_eq!(level.world.random.next_int(), reference.next_int());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_uses_the_same_random_state_for_lazy_and_padded_slots() {
        use std::sync::Arc;
        use sunrise_client::containers::{ModBlockEntity, ModBlockEntityFields, ModContainer};
        use sunrise_core::random::LegacyRandom;

        crate::game::native_entity_refs::tests::with_game(|game| {
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    state.0,
                    Arc::new(ModContainer {
                        slots: 3,
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );
            let mut reference = LegacyRandom::new(7);
            for _ in 0..9 {
                reference.next_double();
            }
            let pos = BlockPos::new(1, 64, 2);
            for len in [3, 0, 1] {
                game.world.random.set_seed(7);
                game.world.block_entities.insert(
                    (pos.x, pos.y, pos.z),
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        "removal:machine".into(),
                        ModBlockEntityFields {
                            items: vec![None; len],
                            ..Default::default()
                        },
                    ))),
                );
                game.with_level(|_, level, _| {
                    level.remove_entity(pos, state, true);
                    assert!(level.spill.spawns.is_empty());
                    assert_eq!(
                        level.world.random.state(),
                        reference.state(),
                        "stored slots: {len}"
                    );
                });
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn removal_spawn_has_exact_position_and_default_delay() {
        removal_outputs(2, |level, _| {
            assert_eq!(level.spill.spawns.len(), 1);
            let spawn = &level.spill.spawns[0];
            assert_eq!(spawn.pos, [1.25, 65.75, -2.5]);
            assert_eq!(spawn.delay, 10);
            assert_eq!(&*spawn.stack.item, "minecraft:diamond");
            assert_eq!(spawn.stack.count, 1);
            assert!(spawn.motion.is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_experience_awards_positioned_orbs() {
        removal_outputs(3, |level, reference| {
            assert_eq!(
                level
                    .spill
                    .orbs
                    .iter()
                    .map(|(_, value, _)| *value)
                    .collect::<Vec<_>>(),
                [7, 3]
            );
            for (at, _, merge) in &level.spill.orbs {
                assert_eq!(*at, [1.25, 65.75, -2.5]);
                assert_eq!(*merge, reference.next_int_bound(40).unwrap());
            }
            assert_eq!(level.world.random.next_int(), reference.next_int());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_empty_slot_draws_three_doubles_without_spawning() {
        removal_outputs(4, |level, reference| {
            // Containers.dropItemStack draws x, y, z before its isEmpty loop.
            for _ in 0..3 {
                reference.next_double();
            }
            // The filled slot follows the empty one, using its own three draws.
            let at = [
                1.125 + reference.next_double() * 0.75,
                64.0 + reference.next_double() * 0.75,
                2.125 + reference.next_double() * 0.75,
            ];
            reference.next_int_bound(21).unwrap();
            for _ in 0..6 {
                reference.next_double();
            }
            assert_eq!(level.spill.spawns.len(), 1);
            assert_eq!(level.spill.spawns[0].pos, at);
            assert_eq!(level.spill.spawns[0].stack.count, 1);
            assert_eq!(level.world.random.next_int(), reference.next_int());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn removal_unanswered_part_leaves_random_untouched() {
        removal_outputs(0, |level, reference| {
            assert!(level.spill.spawns.is_empty());
            assert!(level.spill.orbs.is_empty());
            assert_eq!(level.world.random.next_int(), reference.next_int());
        });
    }

    /// Listens to `block_drops`: takes any break of dirt, adds 2 points to the rest.
    struct DropsMod;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for DropsMod {
        fn block_drops(
            &self,
            _: &dyn mod_api::LevelHost,
            event: &mut mod_api::BlockDropsEvent<'_>,
        ) -> mod_api::Flow {
            if event.drops.iter().any(|d| {
                d.stack().is_some_and(|s| {
                    mod_api::__private::item_key_id(s.item()) == Some("minecraft:dirt")
                })
            }) {
                return mod_api::Flow::Cancel;
            }
            event.experience += 2;
            mod_api::Flow::Continue
        }
    }

    /// `Block.dropResources` builds its item entities (and samples the experience) before
    /// `BlockDropsEvent`, so the level random ends where it does without a listener; a mod that
    /// cancels gets no entities and no experience but the draws still happened; a mod that
    /// edits the amount changes only the amount.
    #[test]
    #[ignore = "daily"]
    fn block_drops_draws_before_the_event_and_cancel_empties_the_break() {
        use std::sync::Arc;
        use sunrise_core::random::LegacyRandom;
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            let ore = registry.default_state("minecraft:diamond_ore").unwrap();
            let pos = BlockPos::new(1, 64, 2);
            let break_of = |game: &mut Game<'_>, state, item, exp_to_drop| {
                game.world.random =
                    sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(7));
                let dropped = game.with_level(|_, level, _| {
                    level.block_dropped(
                        pos,
                        state,
                        None,
                        None,
                        BlockDropEntity::None,
                        vec![ItemStack::new(item, 1)],
                        exp_to_drop,
                    )
                });
                (dropped, game.world.random.next_int())
            };
            let (plain_ore, ore_after) = break_of(game, ore, "minecraft:diamond", None);
            let (plain_dirt, dirt_after) = break_of(game, dirt, "minecraft:dirt", None);
            let (overridden, _) = break_of(game, ore, "minecraft:diamond", Some(17));
            assert_eq!(overridden.experience, 17);
            assert_eq!(plain_dirt.items.len(), 1);
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("block-drops", Box::new(DropsMod))].into();
            mod_platform::testing::with_mods(mods, || {
                let (heard, after) = break_of(game, ore, "minecraft:diamond", None);
                assert_eq!(after, ore_after);
                assert_eq!(heard.items.len(), 1);
                assert_eq!(heard.items[0].position, plain_ore.items[0].position);
                assert_eq!(heard.experience, plain_ore.experience + 2);
                assert!(!heard.canceled);
                let (overridden, _) = break_of(game, ore, "minecraft:diamond", Some(17));
                assert_eq!(overridden.experience, 19);
                let (taken, after) = break_of(game, dirt, "minecraft:dirt", None);
                assert_eq!(after, dirt_after);
                assert!(taken.canceled && taken.items.is_empty() && taken.experience == 0);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn disabled_block_drops_draw_offsets_without_constructing_items() {
        use crate::game::native_entity_refs::tests::with_game;
        use std::sync::{Arc, Mutex};
        use sunrise_client::random_source::record_seeds;

        struct Observe(Arc<Mutex<Vec<usize>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Observe {
            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut mod_api::BlockDropsEvent<'_>,
            ) -> mod_api::Flow {
                self.0.lock().unwrap().push(event.drops.len());
                mod_api::Flow::Continue
            }
        }

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "observe",
            Box::new(Observe(seen.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                assert!(
                    game.ui
                        .rules
                        .set("block_drops", sunrise_client::gamerules::Value::Bool(false),)
                );
                let dirt = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:dirt")
                    .unwrap();
                game.world.random.set_seed(7);
                let (dropped, seeds) = record_seeds(|| {
                    game.with_level(|_, level, _| {
                        level.block_dropped(
                            BlockPos::new(2, 64, 1),
                            dirt,
                            None,
                            None,
                            BlockDropEntity::None,
                            vec![ItemStack::new("minecraft:dirt", 1)],
                            Some(5),
                        )
                    })
                });
                assert!(dropped.items.is_empty());
                assert_eq!(dropped.experience, 0);
                assert!(seeds.complete(), "no entity RNG seed is drawn");
                // Java Random(7) after three nextDouble calls for the offsets.
                assert_eq!(game.world.random.next_int(), -441_191_359);
            });
        });
        assert_eq!(*seen.lock().unwrap(), [0]);
    }

    /// Removing drop 0 shifts drop 1 without stealing drop 0's `ItemRandom`, and a later
    /// listener's invalid stack edit or out-of-bounds edit is rejected per call without rolling
    /// back earlier listeners' valid drop mutations or experience.
    #[test]
    #[ignore = "daily"]
    fn block_drops_per_edit_rejection_and_random_preservation() {
        use mod_api::{BlockDropsEvent, Flow, Hooks, ItemDrop, ModEntry, Mods, StackWriteError};
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use sunrise_core::random::LegacyRandom;

        struct MutatingListener;
        #[mod_api::__subscriptions]
        impl Hooks for MutatingListener {
            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut BlockDropsEvent<'_>,
            ) -> Flow {
                assert_eq!(event.drops.len(), 2);
                assert!(event.drops.remove(0));
                assert_eq!(event.drops.len(), 1);
                event.drops.get_mut(0).unwrap().set_pickup_delay(25);
                let diamond = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:diamond").unwrap(),
                    1,
                )
                .unwrap();
                event
                    .drops
                    .push(ItemDrop::new(
                        diamond,
                        [1.5, 65.0, 2.5],
                        [0.0, 0.2, 0.0],
                        10,
                    ))
                    .unwrap();
                event.experience = 9;
                Flow::Continue
            }
        }

        struct InvalidEditListener(Arc<AtomicBool>);
        #[mod_api::__subscriptions]
        impl Hooks for InvalidEditListener {
            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut BlockDropsEvent<'_>,
            ) -> Flow {
                assert_eq!(event.drops.len(), 2);
                let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
                let invalid = mod_api::ItemStack::new(stone, 65).unwrap();
                let invalid_drop = ItemDrop::new(invalid.clone(), [1.5, 65.0, 2.5], [0.0; 3], 10);
                assert_eq!(
                    event.drops.push(invalid_drop),
                    Err(StackWriteError::CountExceeded { maximum: 64 })
                );
                assert_eq!(
                    event.drops.get_mut(0).unwrap().set_stack(invalid),
                    Err(StackWriteError::CountExceeded { maximum: 64 })
                );
                let valid = ItemDrop::new(
                    mod_api::ItemStack::new(stone, 1).unwrap(),
                    [1.5, 65.0, 2.5],
                    [0.0; 3],
                    10,
                );
                assert_eq!(event.drops.insert(99, valid), Ok(false));
                assert!(!event.drops.remove(99));
                assert_eq!(event.drops.len(), 2);
                let surviving = event.drops.get(0).unwrap();
                assert_eq!(
                    mod_api::__private::item_key_id(surviving.stack().unwrap().item()),
                    Some("minecraft:cobblestone")
                );
                assert_eq!(surviving.pickup_delay(), 25);
                self.0.store(true, Ordering::Relaxed);
                Flow::Continue
            }
        }

        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let pos = BlockPos::new(1, 64, 2);
            let break_of = |game: &mut Game<'_>| {
                game.world.random =
                    sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(7));
                let dropped = game.with_level(|_, level, _| {
                    level.block_dropped(
                        pos,
                        stone,
                        None,
                        None,
                        BlockDropEntity::None,
                        vec![
                            ItemStack::new("minecraft:stone", 1),
                            ItemStack::new("minecraft:cobblestone", 1),
                        ],
                        None,
                    )
                });
                (dropped, game.world.random.next_int())
            };

            let ((baseline, baseline_after), entity_seeds) =
                sunrise_client::random_source::record_seeds(|| break_of(game));
            assert_eq!(baseline.items.len(), 2);
            assert!(baseline.items[0].random.is_some());
            assert!(baseline.items[1].random.is_some());
            assert_ne!(baseline.items[0].random, baseline.items[1].random);

            let ran_second = Arc::new(AtomicBool::new(false));
            let mods: Mods = vec![
                ModEntry::new("first", Box::new(MutatingListener)),
                ModEntry::new("second", Box::new(InvalidEditListener(ran_second.clone()))),
            ]
            .into();
            mod_platform::testing::with_mods(mods, || {
                let (heard, after) =
                    sunrise_client::random_source::replay_seeds(Arc::clone(&entity_seeds), || {
                        break_of(game)
                    });
                assert!(entity_seeds.complete());
                assert!(ran_second.load(Ordering::Relaxed));
                assert_eq!(after, baseline_after);
                assert_eq!(heard.experience, 9);
                assert!(!heard.canceled);
                assert_eq!(heard.items.len(), 2);
                assert_eq!(&*heard.items[0].stack.item, "minecraft:cobblestone");
                assert_eq!(heard.items[0].stack.count, 1);
                assert_eq!(heard.items[0].pickup_delay, 25);
                assert_eq!(heard.items[0].position, baseline.items[1].position);
                assert_eq!(heard.items[0].velocity, baseline.items[1].velocity);
                assert_eq!(heard.items[0].random, baseline.items[1].random);
                assert_ne!(heard.items[0].random, baseline.items[0].random);
                assert_eq!(&*heard.items[1].stack.item, "minecraft:diamond");
                assert_eq!(heard.items[1].stack.count, 1);
                assert_eq!(heard.items[1].position, [1.5, 65.0, 2.5]);
                assert_eq!(heard.items[1].velocity, [0.0, 0.2, 0.0]);
                assert_eq!(heard.items[1].pickup_delay, 10);
                assert_eq!(heard.items[1].random, None);
            });
        });
    }

    /// An empty or unrepresentable optional tool (such as an unregistered item from an older
    /// save) omits only `event.tool` and still dispatches `block_drops` so listeners can edit
    /// drops and experience or cancel the break.
    #[test]
    #[ignore = "daily"]
    fn block_drops_dispatches_with_empty_or_unrepresentable_tool() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use sunrise_core::random::LegacyRandom;

        struct ToolOmitListener(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ToolOmitListener {
            fn block_drops(
                &self,
                _: &dyn mod_api::LevelHost,
                event: &mut mod_api::BlockDropsEvent<'_>,
            ) -> mod_api::Flow {
                self.0.fetch_add(1, Ordering::Relaxed);
                assert!(
                    event.tool.is_none(),
                    "empty or unrepresentable tools omit only event.tool"
                );
                if event.drops.iter().any(|drop| {
                    drop.stack().is_some_and(|stack| {
                        mod_api::__private::item_key_id(stack.item()) == Some("minecraft:dirt")
                    })
                }) {
                    return mod_api::Flow::Cancel;
                }
                event.drops.get_mut(0).unwrap().set_pickup_delay(44);
                event.experience += 5;
                mod_api::Flow::Continue
            }
        }

        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let ore = registry.default_state("minecraft:diamond_ore").unwrap();
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            let pos = BlockPos::new(1, 64, 2);
            let calls = Arc::new(AtomicUsize::new(0));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "tool-omit",
                Box::new(ToolOmitListener(calls.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                for tool in [
                    ItemStack::new("missingmod:ghost_pick", 1),
                    ItemStack::new("minecraft:air", 1),
                    ItemStack::new("minecraft:diamond_pickaxe", 0),
                ] {
                    game.world.random =
                        sunrise_core::random::LevelRandom::for_level(LegacyRandom::new(7));
                    let edited = game.with_level(|_, level, _| {
                        level.block_dropped(
                            pos,
                            ore,
                            None,
                            Some(&tool),
                            BlockDropEntity::None,
                            vec![ItemStack::new("minecraft:diamond", 1)],
                            Some(3),
                        )
                    });
                    assert!(!edited.canceled);
                    assert_eq!(edited.experience, 8);
                    assert_eq!(edited.items.len(), 1);
                    assert_eq!(edited.items[0].pickup_delay, 44);

                    let canceled = game.with_level(|_, level, _| {
                        level.block_dropped(
                            pos,
                            dirt,
                            None,
                            Some(&tool),
                            BlockDropEntity::None,
                            vec![ItemStack::new("minecraft:dirt", 1)],
                            Some(3),
                        )
                    });
                    assert!(canceled.canceled);
                    assert!(canceled.items.is_empty());
                    assert_eq!(canceled.experience, 0);
                }
            });
            assert_eq!(calls.load(Ordering::Relaxed), 6);
        });
    }

    /// A mod block entity's part answers its removal as vanilla's `preRemoveSideEffects`
    /// override does: found by the entity's type, not its block, reading its attachments in
    /// place, its stacks scattered after the block entity's own; skipped under flag 256, and
    /// drawing nothing when it gives nothing back.
    #[test]
    #[ignore = "daily"]
    fn a_block_entity_part_gives_back_what_it_held_when_removed() {
        use mod_api::Holder;
        use mod_platform::__private::part_tests::{HOARD, HOARD_ASKED, game_thread};
        use std::sync::Arc;
        use std::sync::atomic::Ordering;
        use sunrise_client::chunk::Chunk;
        use sunrise_client::containers::{ModBlockEntity, ModBlockEntityFields};
        crate::game::native_entity_refs::tests::with_game(|game| {
            let _mods = game_thread();
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            // Each case: the entity's type, its hoard, side effects; the gold that comes out.
            let asked = HOARD_ASKED.load(Ordering::Relaxed);
            for (id, hoard, side_effects, gold) in [
                ("doubler:hoard", 5, true, 5),
                ("doubler:hoard", 5, false, 0),
                ("doubler:hoard", 0, true, 0),
                // A different declared type's removal doesn't reach `Hoard`.
                ("doubler:press_entity", 5, true, 0),
            ] {
                let pos = BlockPos::new(1, 64, 2);
                let key = (pos.x, pos.y, pos.z);
                let entity = BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    id.into(),
                    ModBlockEntityFields::default(),
                )));
                game.world.block_entities.insert(key, entity);
                let holder = Holder::BlockEntity([pos.x, pos.y, pos.z]);
                let host: &mut dyn mod_api::Host = game;
                HOARD.set(host, holder, hoard).unwrap();
                let (spawns, random) = game.with_level(|_, level, _| {
                    let mut scatter = level.world.random.clone();
                    level.remove_entity(pos, stone, side_effects);
                    // The game's own draws for what came out, from the random as it was.
                    let count = level.spill.spawns.iter().map(|s| s.stack.count).sum();
                    if count > 0 {
                        removal::drop_item_stack(&mut scatter, pos, count);
                    }
                    let spawns: Vec<_> = level
                        .spill
                        .spawns
                        .iter()
                        .map(|s| (s.stack.item.to_string(), s.stack.count))
                        .collect();
                    (spawns, scatter.next_int() == level.world.random.next_int())
                });
                let total: u32 = spawns.iter().map(|(_, count)| count).sum();
                assert!(
                    spawns
                        .iter()
                        .all(|(item, _)| item == "minecraft:gold_ingot")
                );
                assert_eq!(
                    total, gold,
                    "{id} holding {hoard}, side effects {side_effects}"
                );
                assert!(random, "{id}: the random drew only the scatter");
                assert!(game.world.block_entities.get(&key).is_none());
            }
            // Only the two `doubler:hoard` removals with side effects asked it.
            assert_eq!(HOARD_ASKED.load(Ordering::Relaxed) - asked, 2);
        });
    }

    /// LevelChunk ticks the resident row and calls its pre-removal effects before retiring it.
    /// Replacing its own block must spill both the container contents and the Part's gold.
    #[test]
    #[ignore = "daily"]
    fn a_block_entity_removing_itself_in_its_tick_runs_resident_removal() {
        use mod_api::Holder;
        use mod_platform::__private::part_tests::{HOARD, HOARD_ASKED};
        use std::sync::Arc;
        use std::sync::atomic::Ordering;
        use sunrise_client::chunk::Chunk;
        use sunrise_client::containers::{ModBlockEntity, ModBlockEntityFields, ModContainer};
        let Some(root) = crate::game::native_entity_refs::tests::declared_part_root(
            "game::spill::tests::a_block_entity_removing_itself_in_its_tick_runs_resident_removal",
        ) else {
            return;
        };
        crate::game::native_entity_refs::tests::with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = &game.pack.assets.registry;
            let hoard = registry.default_state("test:hoard").unwrap();
            let granite = registry.default_state("minecraft:granite").unwrap();
            let pos = BlockPos::new(1, 64, 2);
            let key = (pos.x, pos.y, pos.z);
            game.world.set(pos.x, pos.y, pos.z, hoard).unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    hoard.0,
                    Arc::new(ModContainer {
                        slots: 1,
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );
            // Data `[1]` makes the hoard's tick set its own block to granite.
            let fields = ModBlockEntityFields {
                items: vec![Some(ItemStack::new("minecraft:diamond", 3))],
                data: vec![1],
                ..Default::default()
            };
            game.world.block_entities.insert(
                key,
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "doubler:hoard".into(),
                    fields,
                ))),
            );
            let holder = Holder::BlockEntity([pos.x, pos.y, pos.z]);
            let host: &mut dyn mod_api::Host = game;
            HOARD.set(host, holder, 5).unwrap();
            let asked = HOARD_ASKED.load(Ordering::Relaxed);
            let dropped = game.dropped.len();
            assert!(game.tick_mod_block_entity(key));
            assert_eq!(game.world.block(pos.x, pos.y, pos.z), granite);
            assert_eq!(HOARD_ASKED.load(Ordering::Relaxed), asked + 1);
            assert!(game.world.block_entities.get(&key).is_none());
            assert!(game.world.block_entities.attachments(key).is_none());
            let out: Vec<_> = game.dropped[dropped..]
                .iter()
                .map(|d| (d.stack.item.to_string(), d.stack.count))
                .collect();
            assert_eq!(
                out,
                [
                    ("minecraft:diamond".to_owned(), 3),
                    ("minecraft:gold_ingot".to_owned(), 5),
                ]
            );
        });
    }
    use sunrise_client::{block_entity_item, item_data, mobs::raider, save};
    use sunrise_core::nbt::Tag;

    #[test]
    fn a_used_run_is_retold_in_place() {
        let spawn = |item: &str, x: f64| Spawn {
            random: None,
            stack: ItemStack::new(item, 1),
            pos: [x, 0.0, 0.0],
            motion: None,
            delay: 10,
        };
        let mut spawns = vec![
            spawn("minecraft:stone", 0.0),
            spawn("minecraft:sweet_berries", 1.0),
            spawn("minecraft:sweet_berries", 2.0),
            spawn("minecraft:dirt", 3.0),
        ];
        let used = [UsedDrops {
            start: 1,
            end: 3,
            pos: [4, 64, 4],
            state: BlockStateId(0),
            table: "minecraft:harvest/sweet_berry_bush",
        }];
        retell_runs(&mut spawns, &used, |used, origin, drops| {
            assert_eq!(used.table, "minecraft:harvest/sweet_berry_bush");
            assert_eq!(origin, [4.5, 64.5, 4.5]);
            assert_eq!(drops.len(), 2);
            drops.remove(0);
            drops.push(ItemStack::new("minecraft:diamond", 1));
        });
        let items: Vec<(&str, f64)> = spawns.iter().map(|s| (&*s.stack.item, s.pos[0])).collect();
        assert_eq!(
            items,
            [
                ("minecraft:stone", 0.0),
                ("minecraft:sweet_berries", 1.0),
                ("minecraft:diamond", 2.0),
                ("minecraft:dirt", 3.0),
            ]
        );
    }

    #[test]
    fn tool_source_keeps_the_full_patch_and_distinguishes_stored_enchantments() {
        let potion = Tag::compound([("potion", Tag::String("minecraft:healing".into()))]);
        let stored = Tag::compound([("minecraft:sharpness", Tag::Int(13))]);
        let patch = Tag::compound([
            ("minecraft:potion_contents", potion.clone()),
            ("minecraft:stored_enchantments", stored.clone()),
            ("!minecraft:unbreakable", Tag::Compound(Default::default())),
        ]);
        let (damage, extra) = item_data::decode_components(Some(&patch));
        let tool = ItemStack {
            item: std::sync::Arc::from("minecraft:enchanted_book"),
            count: 1,
            damage,
            extra,
        };
        let snapshot = crate::game::creatures::to_loot(&tool);
        assert!(snapshot.enchantments.is_empty());
        let prototype = [
            (
                "minecraft:unbreakable".into(),
                Tag::Compound(Default::default()),
            ),
            (
                "minecraft:enchantments".into(),
                Tag::Compound(Default::default()),
            ),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            sunrise_world::loot::copy::stack_components(&snapshot, Some(&prototype)),
            [
                ("minecraft:potion_contents".into(), potion),
                ("minecraft:stored_enchantments".into(), stored),
                (
                    "minecraft:enchantments".into(),
                    Tag::Compound(Default::default())
                )
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn ominous_banner_survives_placement_save_and_drop() {
        let original = raider::ominous_banner();
        let Tag::Compound(components) = item_data::component_snapshot(&original).unwrap() else {
            panic!("banner components");
        };
        for block in ["minecraft:white_banner", "minecraft:white_wall_banner"] {
            let pos = (1, 64, 2);
            let fresh = BlockEntity::for_block(block).unwrap();
            let fresh = save::block_entity_encode::encode(pos, &fresh, block, &Default::default());
            let placed = block_entity_item::place(&fresh, &components, false);
            let (_, placed) = save::decode_block_entity(&placed).unwrap();
            let saved = save::block_entity_encode::encode(pos, &placed, block, &Default::default());
            let (_, loaded) = save::decode_block_entity(&saved).unwrap();
            let BlockEntity::Banner(banner) = &loaded else {
                panic!("placed banner");
            };
            assert_eq!(banner.layers.len(), item_data::OMINOUS_LAYERS.len());
            let mut pending = Pending::default();
            pending.record_saved([pos.0, pos.1, pos.2], &saved);
            let mut items = sunrise_client::items::Items::for_tests(Default::default());
            items.set_loot_tables(std::sync::Arc::new([(
                "test:banner".to_owned(),
                serde_json::json!({"type":"minecraft:empty","pools":[{"rolls":1,"entries":[{
                    "type":"minecraft:item","name":"minecraft:white_banner",
                    "modifier":{"type":"minecraft:copy_components","source":"block_entity",
                        "include":["minecraft:banner_patterns","minecraft:custom_name",
                            "minecraft:item_name","minecraft:rarity","minecraft:tooltip_display"]}
                }]}]}),
            )].into_iter().collect()));
            let source = pending.source([pos.0, pos.1, pos.2]);
            let dropped = sunrise_world::loot::copy::with_block_source(source.as_deref(), || {
                items
                    .loot_table(
                        "test:banner",
                        false,
                        None,
                        &mut sunrise_core::random::LegacyRandom::new(0),
                    )
                    .remove(0)
            });
            assert!(dropped.data().unwrap().ominous_banner);
            assert!(sunrise_client::banner::tooltip_layers(&dropped).is_empty());
            assert_eq!(
                sunrise_client::item_tooltip::name_line(&dropped, &|_| None),
                "Ominous Banner"
            );
            assert_eq!(
                sunrise_client::item_tooltip::rarity(&dropped, "common"),
                "uncommon"
            );
            assert_eq!(
                item_data::component_snapshot(&dropped),
                Some(Tag::Compound(components.clone()))
            );
        }
    }
}

#[cfg(test)]
mod crafter_drop_name_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn actual_crafter_destruction_keeps_source_name_off_the_dropped_item() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            game.block_ticks
                .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
            assert!(
                game.ui
                    .rules
                    .bool_at(sunrise_client::gamerules::vanilla::BLOCK_DROPS)
            );
            let clock = game.world.game_time;
            let mut unexpected = Vec::new();
            // Positive name-copy and unnamed controls precede the assigned case.
            for (x, z, kind, name) in [
                (3, 11, "chest", Some("Box")),
                (11, 11, "shulker_box", Some("Box")),
                (11, 3, "crafter", None),
                (3, 3, "crafter", Some("Rig")),
            ] {
                let extra = name.map_or(String::new(), |n| format!(",CustomName:\"{n}\""));
                assert!(game.run_command(&format!(
                "setblock {x} 200 {z} minecraft:{kind}{{Items:[{{Slot:0b,id:\"minecraft:diamond\",count:3,components:{{\"minecraft:custom_data\":{{h212:1}}}}}}]{extra}}} strict"
            )));
                let key = (x, 200, z);
                let item_id = format!("minecraft:{kind}");
                assert_eq!(
                    &*game.res.facts().get(game.world.block(x, 200, z)).name,
                    item_id
                );
                assert_eq!(
                    &*game.res.facts().get(game.world.block(x, 199, z)).name,
                    "minecraft:stone"
                );
                assert_eq!(
                    game.world.block_names.get(&key),
                    name.map(|n| Tag::String(n.into())).as_ref()
                );
                let entity = game
                    .world
                    .block_entities
                    .get(&key)
                    .expect("actual source block entity");
                assert_eq!(matches!(entity, BlockEntity::Crafter(_)), kind == "crafter");
                let diamond = entity.slots()[0].as_ref().expect("actual loaded inventory");
                assert_eq!((&*diamond.item, diamond.count), ("minecraft:diamond", 3));
                assert_eq!(
                    diamond
                        .data()
                        .and_then(|d| d.other("minecraft:custom_data"))
                        .and_then(|t| t.get("h212")),
                    Some(&Tag::Int(1))
                );
                assert!(game.world.loaded(x, z), "actual destruction-level chunk");
                assert!(game.run_command(&format!(
                "execute if data block {x} 200 {z} {{id:\"minecraft:{kind}\",x:{x},y:200,z:{z}}}"
            )));
                if let Some(n) = name {
                    assert!(game.run_command(&format!(
                        "execute if data block {x} 200 {z} {{CustomName:\"{n}\"}}"
                    )));
                } else {
                    assert!(
                        game.run_command(&format!(
                            "execute unless data block {x} 200 {z} CustomName"
                        ))
                    );
                }
                let start = game.dropped.len();
                assert!(game.run_command(&format!("setblock {x} 200 {z} air destroy")));
                game.flush_spill();
                assert!(game.res.facts().is_air(game.world.block(x, 200, z)));
                assert!(!game.world.block_entities.contains_key(&key));
                assert!(!game.world.block_names.contains_key(&key));
                let emitted = &game.dropped[start..];
                let matching: Vec<_> = emitted
                    .iter()
                    .filter(|d| *d.stack.item == item_id)
                    .collect();
                assert_eq!(matching.len(), 1, "one actual block item: {kind}");
                let drop = matching[0];
                assert_eq!(drop.stack.count, 1);
                assert_eq!((drop.age, drop.pickup_delay), (0, 10));
                let actual = drop.stack.data().and_then(|d| d.custom_name.as_ref());
                if kind == "crafter" {
                    if actual.is_some() {
                        unexpected.push((name, actual.cloned()));
                    }
                } else {
                    assert_eq!(
                        actual,
                        Some(&Tag::String("Box".into())),
                        "positive ordinary name-copy control"
                    );
                }
                if kind == "shulker_box" {
                    let contents = &drop.stack.data().unwrap().container;
                    assert_eq!(contents.len(), 1);
                    let kept = &contents[0].1;
                    assert_eq!((&*kept.item, kept.count), ("minecraft:diamond", 3));
                    assert_eq!(
                        kept.data()
                            .and_then(|d| d.other("minecraft:custom_data"))
                            .and_then(|t| t.get("h212")),
                        Some(&Tag::Int(1))
                    );
                    assert_eq!(emitted.len(), 1);
                } else {
                    let contents: Vec<_> = emitted
                        .iter()
                        .filter(|d| &*d.stack.item == "minecraft:diamond")
                        .collect();
                    assert_eq!(contents.len(), 1);
                    assert_eq!(contents[0].stack.count, 3);
                    assert_eq!(
                        contents[0]
                            .stack
                            .data()
                            .and_then(|d| d.other("minecraft:custom_data"))
                            .and_then(|t| t.get("h212")),
                        Some(&Tag::Int(1))
                    );
                    assert_eq!(emitted.len(), 2);
                }
                assert_eq!(game.world.game_time, clock, "zero observation ticks");
            }
            assert!(
                unexpected.is_empty(),
                "actual vanilla crafter drop retained source name: {unexpected:?}"
            );
        });
    }
}

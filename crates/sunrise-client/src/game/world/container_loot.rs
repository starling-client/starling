//! First-access loot shared by blocks and chest vehicles.
use super::Game;
use sunrise_client::{containers::BlockEntity, save::PendingContainerLoot};
use sunrise_core::{nbt::Tag, random::LegacyRandom};

#[derive(Clone, Copy)]
pub(super) enum LootTarget {
    Block((i32, i32, i32)),
    Cart(u32),
    Boat(u32),
}
impl Game<'_> {
    /// Whether the container at `key` still waits with a loot table.
    pub(super) fn block_loot_pending(&self, key: (i32, i32, i32)) -> bool {
        self.world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|c| c.saved.as_ref())
            .is_some_and(|s| s.pending_loot.contains_key(&key))
    }
    pub(super) fn loot_inventory_item(
        &mut self,
        key: (i32, i32, i32),
        index: usize,
    ) -> Option<sunrise_client::items::ItemStack> {
        self.unpack_loot(LootTarget::Block(key), None);
        self.world
            .block_entities
            .get(&key)?
            .slots()
            .get(index)?
            .clone()
    }
    /// A hopper cart's tick owns its metadata while its resident row is lent out.
    /// Publish only a pending inventory for this access, then recover the result.
    pub(super) fn unpack_ticked_cart_loot(
        &mut self,
        cart: &mut super::minecarts::Minecart,
    ) -> bool {
        if cart.loot.is_none() {
            return true;
        }
        let Some(i) = self
            .carts
            .iter()
            .position(|c| c.id == cart.id && c.join == cart.join)
        else {
            return false;
        };
        self.carts.update(i, |owner| {
            owner.loot = cart.loot.take();
            owner.items.clone_from(&cart.items);
            owner.position = cart.position;
        });
        self.unpack_loot(LootTarget::Cart(cart.id), None);
        let Some(i) = self
            .carts
            .iter()
            .position(|c| c.id == cart.id && c.join == cart.join)
        else {
            return false;
        };
        self.carts.update(i, |owner| {
            cart.loot = owner.loot.take();
            cart.items.clone_from(&owner.items);
        });
        true
    }
    /// Consumes metadata before callbacks, so reentrant access cannot roll twice.
    /// A player context is captured from the admitted actor, also during as_guest.
    pub(super) fn unpack_loot(&mut self, target: LootTarget, player: Option<mod_api::PlayerRef>) {
        let (pending, pos) = match target {
            LootTarget::Block(key) => {
                let Some(pending) = take_block_pending(&mut self.world, key) else {
                    return;
                };
                if mixed_state_dropped(&self.world.block_entities[&key]) {
                    return;
                }
                (pending, key)
            }
            LootTarget::Cart(id) => {
                let Some(i) = self.carts.iter().position(|c| c.id == id) else {
                    return;
                };
                let Some(pending) = self.carts.update(i, |c| c.loot.take()) else {
                    return;
                };
                self.store_cart_items((i32::MAX - 15, super::minecarts::CART_KEY_Y, id as i32));
                let c = &self.carts[i];
                (
                    pending,
                    (
                        c.position.x.floor() as i32,
                        c.position.y.floor() as i32,
                        c.position.z.floor() as i32,
                    ),
                )
            }
            LootTarget::Boat(id) => {
                let Some(i) = self.boats.iter().position(|b| b.id == id) else {
                    return;
                };
                let Some(pending) = self
                    .persist
                    .raw
                    .get(&id)
                    .and_then(PendingContainerLoot::from_tag)
                else {
                    return;
                };
                if let Some(Tag::Compound(map)) = self.persist.raw.get_mut(&id) {
                    map.remove(&"LootTable".into());
                    map.remove(&"LootTableSeed".into());
                }
                self.store_cart_items((i32::MAX - 15, super::minecarts::CART_KEY_Y, id as i32));
                let b = &self.boats[i];
                (
                    pending,
                    (
                        b.position.x.floor() as i32,
                        b.position.y.floor() as i32,
                        b.position.z.floor() as i32,
                    ),
                )
            }
        };
        // ContainerEntity unpacks the table even when some vehicle slots are taken.
        // LootTable.fill chooses only the free slots.
        let table = pending.table_id();
        if player.is_some() {
            self.trigger_loot_table(&table);
        }
        let Some(loot) = self.server_loot.get() else {
            return;
        };
        let biome = self
            .biome_name_at(super::Vec3::new(pos.0 as f64, pos.1 as f64, pos.2 as f64))
            .unwrap_or_default();
        let mut entity = player.map(|_| self.killer_loot_entity(None));
        if let Some(entity) = &mut entity {
            entity.on_fire = self
                .player
                .as_ref()
                .is_some_and(sunrise_client::motion::PlayerMotion::is_on_fire);
        }
        let luck = if player.is_some() {
            self.player
                .as_mut()
                .and_then(|p| p.attribute("minecraft:luck").ok())
                .unwrap_or(0.0) as f32
        } else {
            0.0
        };
        let Some(mut slots) = (match target {
            LootTarget::Block(key) => self
                .world
                .block_entities
                .get(&key)
                .map(|e| e.slots().to_vec()),
            LootTarget::Cart(id) => self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.items.clone()),
            LootTarget::Boat(id) => self
                .boats
                .iter()
                .find(|b| b.id == id)
                .map(|b| b.items.clone()),
        }) else {
            return;
        };
        let origin = match target {
            LootTarget::Block(_) => [pos.0 as f64 + 0.5, pos.1 as f64 + 0.5, pos.2 as f64 + 0.5],
            LootTarget::Cart(id) => {
                let p = self.carts.iter().find(|c| c.id == id).unwrap().position;
                [p.x, p.y, p.z]
            }
            LootTarget::Boat(id) => {
                let p = self.boats.iter().find(|b| b.id == id).unwrap().position;
                [p.x, p.y, p.z]
            }
        };
        let mut random = self.world.random.clone();
        let mut sequences = std::mem::take(&mut self.rand_sequences);
        // ORIGIN is the container's centre (a vehicle's position); its location checks
        // read the level.
        let rolled = {
            let level = crate::game::advancements::View(&*self);
            let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                sunrise_client::advancements::location(predicate, at, &level)
            };
            let context = sunrise_world::loot::ContainerLootContext {
                biome: Some(&biome),
                location: Some(sunrise_client::advancements::loot_location(origin, &read)),
                luck,
                entity: entity.as_ref(),
            };
            roll_fill(
                &pending,
                &loot,
                &mut random,
                &mut sequences,
                self.level.seed,
                context,
            )
        };
        if let Some((mut items, source)) = rolled {
            self.container_fill_hook(&table, target, origin, player, &mut items);
            place_fill(
                items,
                &mut slots,
                &loot,
                source,
                &mut random,
                &mut sequences,
                self.level.seed,
            );
        }
        self.world.random = random;
        self.rand_sequences = sequences;
        match target {
            LootTarget::Block(key) => {
                if let Some(e) = self.world.block_entities.get_mut(&key) {
                    e.replace_slots(&slots);
                }
            }
            LootTarget::Cart(id) => {
                if let Some(i) = self.carts.iter().position(|c| c.id == id) {
                    self.carts.update(i, |c| c.items = slots);
                }
            }
            LootTarget::Boat(id) => {
                if let Some(i) = self.boats.iter().position(|b| b.id == id) {
                    self.boats.update(i, |b| b.items = slots);
                }
            }
        }
        let id = match target {
            LootTarget::Cart(id) | LootTarget::Boat(id) => Some(id),
            _ => None,
        };
        if let Some(id) = id {
            let items = match target {
                LootTarget::Cart(_) => self
                    .carts
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.items.clone()),
                _ => self
                    .boats
                    .iter()
                    .find(|b| b.id == id)
                    .map(|b| b.items.clone()),
            };
            if let (Some(items), Some(BlockEntity::Chest(open))) = (
                items,
                self.world.block_entities.get_mut(&(
                    i32::MAX - 15,
                    super::minecarts::CART_KEY_Y,
                    id as i32,
                )),
            ) {
                *open = items;
            }
        }
    }
}

/// Consume the sole block-owned marker before any callback or inventory mutation.
pub(super) fn take_block_pending(
    world: &mut sunrise_client::world::World,
    key: (i32, i32, i32),
) -> Option<PendingContainerLoot> {
    if !world.block_entities.contains_key(&key)
        || !world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|c| c.saved.as_ref())
            .is_some_and(|s| s.pending_loot.contains_key(&key))
    {
        return None;
    }
    let mut pending = None;
    world.with_chunk_extras(key, |extras| pending = extras.pending_loot.remove(&key));
    world.modified.insert((key.0 >> 4, key.2 >> 4));
    pending
}

/// Keep the block-container rule: a vanilla container's table coexisting with items is
/// dropped. A mod container reaches that state through `RandomizableContainer.setLootTable` on
/// a filled container, which vanilla rolls as usual: `LootTable.fill` fills the empty slots.
fn mixed_state_dropped(entity: &BlockEntity) -> bool {
    !matches!(entity, BlockEntity::Mod(_)) && entity.slots().iter().any(Option::is_some)
}

/// The random a container's loot drew from (`LootContext.Builder.create`: the explicit seed,
/// the table's named random sequence, then the level random), kept for the slot shuffle.
enum FillRandom {
    Seeded(LegacyRandom),
    Sequence(String),
    Level,
}

/// `LootTable.fill`'s roll of the pending table (`getRandomItems` with the stack splitter),
/// with `context` read by its conditions. None for a missing table, which draws nothing.
fn roll_fill(
    pending: &PendingContainerLoot,
    loot: &sunrise_world::loot::LootData,
    level_random: &mut LegacyRandom,
    sequences: &mut sunrise_client::random_sequences::RandomSequences,
    world_seed: i64,
    context: sunrise_world::loot::ContainerLootContext<'_>,
) -> Option<(Vec<sunrise_world::loot::LootStack>, FillRandom)> {
    let table = pending.table_id();
    if let Some(seed) = pending.seed_value().filter(|seed| *seed != 0) {
        let mut random = LegacyRandom::new(seed);
        let items = loot.roll_container_with(&table, &mut random, context)?;
        Some((items, FillRandom::Seeded(random)))
    } else if let Some(id) = loot.random_sequence(&table) {
        let items = loot.roll_container_with(&table, sequences.get(id, world_seed), context)?;
        Some((items, FillRandom::Sequence(id.to_owned())))
    } else {
        let items = loot.roll_container_with(&table, level_random, context)?;
        Some((items, FillRandom::Level))
    }
}

/// `LootTable.fill`'s placement into the free slots, drawing on from the random the roll
/// used. Entry points discard a pre-existing mixed state; items added by the player's
/// advancement rewards keep their slots while this fills the free ones.
fn place_fill(
    items: Vec<sunrise_world::loot::LootStack>,
    slots: &mut [Option<sunrise_client::items::ItemStack>],
    loot: &sunrise_world::loot::LootData,
    random: FillRandom,
    level_random: &mut LegacyRandom,
    sequences: &mut sunrise_client::random_sequences::RandomSequences,
    world_seed: i64,
) {
    let mut placed = vec![None; slots.len()];
    let free = slots
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.is_none().then_some(i))
        .collect();
    match random {
        FillRandom::Seeded(mut random) => {
            loot.place_container_loot(items, &mut placed, free, &mut random)
        }
        FillRandom::Sequence(id) => {
            loot.place_container_loot(items, &mut placed, free, sequences.get(&id, world_seed))
        }
        FillRandom::Level => loot.place_container_loot(items, &mut placed, free, level_random),
    }
    for (slot, stack) in slots.iter_mut().zip(placed) {
        if let Some(stack) = stack {
            *slot = Some(sunrise_client::structure_blocks::loot_item_with_components(
                &stack,
            ));
        }
    }
}

impl super::GameLevel<'_> {
    /// Null-player accesses inside world behavior (comparator, destruction).
    pub(super) fn unpack_loot(&mut self, target: LootTarget, player: Option<mod_api::PlayerRef>) {
        debug_assert!(player.is_none());
        if let LootTarget::Cart(id) = target {
            let Some(i) = self.carts.iter().position(|c| c.id == id) else {
                return;
            };
            let Some(pending) = self.carts.update(i, |c| c.loot.take()) else {
                return;
            };
            let Some(loot) = self.loot.as_ref() else {
                return;
            };
            let p = self.carts[i].position;
            let pos = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
            let biome = self.world.chunks.get(&(pos.0 >> 4, pos.2 >> 4)).map(|c| {
                self.biomes
                    .get(c.biome(pos.0 & 15, pos.1, pos.2 & 15))
                    .name
                    .clone()
            });
            let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, id as i32);
            let mut slots = self.carts[i].items.clone();
            if let Some(BlockEntity::Chest(open)) = self.world.block_entities.get(&key) {
                slots.clone_from(open);
            }
            if slots.iter().any(Option::is_some) {
                self.carts.update(i, |cart| cart.items = slots);
                return;
            }
            let loot = loot.clone();
            let mut random = self.world.random.clone();
            let mut sequences = std::mem::take(self.rand_sequences);
            let origin = [p.x, p.y, p.z];
            let rolled = {
                let level = self.location_view();
                let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                    sunrise_client::advancements::location(predicate, at, &level)
                };
                let context = sunrise_world::loot::ContainerLootContext {
                    biome: biome.as_deref(),
                    location: Some(sunrise_client::advancements::loot_location(origin, &read)),
                    ..Default::default()
                };
                roll_fill(
                    &pending,
                    &loot,
                    &mut random,
                    &mut sequences,
                    self.seed,
                    context,
                )
            };
            if let Some((mut items, source)) = rolled {
                self.container_fill_hook(&pending.table_id(), target, origin, player, &mut items);
                place_fill(
                    items,
                    &mut slots,
                    &loot,
                    source,
                    &mut random,
                    &mut sequences,
                    self.seed,
                );
            }
            self.world.random = random;
            *self.rand_sequences = sequences;
            self.carts.update(i, |cart| cart.items = slots);
            if let Some(BlockEntity::Chest(open)) = self.world.block_entities.get_mut(&key) {
                open.clone_from(&self.carts[i].items);
            }
            return;
        }
        let LootTarget::Block(key) = target else {
            unreachable!("level inventory access")
        };
        let Some(pending) = take_block_pending(self.world, key) else {
            return;
        };
        let Some(loot) = self.loot.as_ref() else {
            return;
        };
        let biome = self.world.chunks.get(&(key.0 >> 4, key.2 >> 4)).map(|c| {
            self.biomes
                .get(c.biome(key.0 & 15, key.1, key.2 & 15))
                .name
                .clone()
        });
        let Some(entity) = self.world.block_entities.get(&key) else {
            return;
        };
        if mixed_state_dropped(entity) {
            return;
        }
        let mut slots = entity.slots().to_vec();
        let loot = loot.clone();
        let mut random = self.world.random.clone();
        let mut sequences = std::mem::take(self.rand_sequences);
        let origin = [key.0, key.1, key.2].map(|c| f64::from(c) + 0.5);
        let rolled = {
            let level = self.location_view();
            let read = |predicate: &serde_json::Value, at: [f64; 3]| {
                sunrise_client::advancements::location(predicate, at, &level)
            };
            let context = sunrise_world::loot::ContainerLootContext {
                biome: biome.as_deref(),
                location: Some(sunrise_client::advancements::loot_location(origin, &read)),
                ..Default::default()
            };
            roll_fill(
                &pending,
                &loot,
                &mut random,
                &mut sequences,
                self.seed,
                context,
            )
        };
        if let Some((mut items, source)) = rolled {
            self.container_fill_hook(&pending.table_id(), target, origin, player, &mut items);
            place_fill(
                items,
                &mut slots,
                &loot,
                source,
                &mut random,
                &mut sequences,
                self.seed,
            );
        }
        self.world.random = random;
        *self.rand_sequences = sequences;
        if let Some(entity) = self.world.block_entities.get_mut(&key) {
            entity.replace_slots(&slots);
        }
    }
}

#[cfg(test)]
#[path = "container_loot/tests.rs"]
mod tests;

impl super::GameLevel<'_> {
    /// The other lists stay shared while carts can be mutated by inventory reads.
    pub(super) fn entities(&self) -> super::entity::EntityLists<'_> {
        super::entity::EntityLists {
            carts: &*self.carts,
            ..self.other_entities
        }
    }
    /// Detector rails prioritize command carts, then read the first container.
    pub(super) fn detector_rail_signal(
        &mut self,
        pos: sunrise_core::position::BlockPos,
        powered: bool,
    ) -> i32 {
        use sunrise_core::geometry::{Aabb, Vec3};
        if !powered {
            return 0;
        }
        let (x, y, z) = (pos.x as f64, pos.y as f64, pos.z as f64);
        let area = Aabb::new(
            Vec3::new(x + 0.2, y, z + 0.2),
            Vec3::new(x + 0.8, y + 0.8, z + 0.8),
        );
        let mut container = None;
        for (i, ((lo, hi), cart)) in self.cart_boxes.iter().zip(self.carts.iter()).enumerate() {
            if !area.intersects(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )) {
                continue;
            }
            match cart.kind {
                super::minecarts::CartKind::CommandBlock => return cart.command.success_count,
                super::minecarts::CartKind::Chest | super::minecarts::CartKind::Hopper
                    if container.is_none() =>
                {
                    container = Some(i)
                }
                _ => {}
            }
        }
        let Some(i) = container else { return 0 };
        self.unpack_loot(LootTarget::Cart(self.carts[i].id), None);
        let cart = &self.carts[i];
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, cart.id as i32);
        let slots = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Chest(open)) => open,
            _ => &cart.items,
        };
        super::minecarts::Minecart::container_signal(slots, self.items)
    }
}

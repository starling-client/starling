//! The game's own item capability (`mod_api::ITEMS`, NeoForge's `Capabilities.Item.BLOCK`):
//! container slots and the composter's state-backed input/output, by the rules hoppers follow.
//! Side-local slots are numbered from zero. It acts where the world is simulated, never on
//! a client's copy, and only on the level in place.
use super::{Game, Insertion, Kind, MoveIn, PlacementAttempt, Pos, Slots, View};
use crate::game::{decorated_pot::DECORATED_POT, transactions::After};
use mod_api::{
    BlockCapabilitySite, BlockSide, ItemResource, LevelReadError, LevelRef, Transaction,
};
use sunrise_client::{containers::BlockEntity, items::ItemStack, save};
use sunrise_core::geometry::Direction;
use sunrise_world::registry::BlockStateId;

impl View {
    /// Composter capability ports keep one slot as their level changes; hopper views retain
    /// vanilla's level-dependent slot enumeration.
    fn capability_slots(&self, face: Option<Direction>) -> Slots<'_> {
        if matches!(self.kind, Kind::Composter(_)) {
            Slots::Range(0..1)
        } else {
            self.slots_for_face_iter(face)
        }
    }
}

/// What the game offers without looking at any stack contents or walking a double chest.
enum ItemTarget {
    Composter(i32),
    Pot,
    Container,
}

/// `Level.isInValidBounds` and the resident-only `getChunkNow` / `getChunk(..., false)` query.
fn valid_resident_position(world: &sunrise_client::world::World, [x, y, z]: [i32; 3]) -> bool {
    // `ChunkPyramid.MAX_CHUNK_COORDINATE_VALUE`: the packed BlockPos's 26-bit horizontal
    // bound in chunks, less its safety margin. FULL's accumulated dependency list has size 12.
    const MAX_CHUNK: u32 = (1 << 21) - 1 - (32 + 12 + 1) * 2;
    ((x >> 4).unsigned_abs().max((z >> 4).unsigned_abs()) <= MAX_CHUNK)
        && world.is_loaded_at(x, y, z)
}

impl super::Containers<'_> {
    fn item_target(
        &self,
        state: BlockStateId,
        side: Option<Direction>,
        entity: Option<&BlockEntity>,
    ) -> Option<ItemTarget> {
        match self.name(state) {
            // NeoForge `ComposterWrapper.get`: no unsided or horizontal port. Inactive
            // ports use the level-7 container's transfer rules but still have a handler.
            "minecraft:composter" => {
                let level = self.prop(state, "level")?.parse().ok()?;
                match side {
                    Some(Direction::Up) if level < 7 => Some(ItemTarget::Composter(level)),
                    Some(Direction::Down) if level == 8 => Some(ItemTarget::Composter(8)),
                    Some(Direction::Up | Direction::Down) => Some(ItemTarget::Composter(7)),
                    _ => None,
                }
            }
            // The pot's implicit entity and saved slot use the block's resident state, not
            // `world.block_entities`. A new empty pot need not have a saved tag yet.
            DECORATED_POT => Some(ItemTarget::Pot),
            _ => match entity? {
                BlockEntity::Mod(_) => self
                    .world
                    .block_entities
                    .mod_container(state.0)
                    .map(|_| ItemTarget::Container),
                entity => (!entity.slots().is_empty()).then_some(ItemTarget::Container),
            },
        }
    }

    pub(in crate::game) fn capability_site(
        &self,
        at: BlockSide,
        state: BlockStateId,
        entity_removed: bool,
    ) -> Option<BlockCapabilitySite> {
        use crate::game::{api_values::TryIntoApi, native_block_entity_refs::block_entity_handle};
        if !valid_resident_position(self.world, at.pos) {
            return None;
        }
        let [x, y, z] = at.pos;
        let entity = if entity_removed {
            None
        } else {
            self.world.block_entities.get(&(x, y, z))
        };
        let resident = entity.and_then(|entity| {
            let handle = block_entity_handle(self.world, at.level, at.pos)?;
            let kind = mod_api::__private::block_entity_type_key(save::block_entity_id(
                entity,
                self.name(state),
            ));
            Some((handle, kind))
        });
        Some(BlockCapabilitySite::new(
            at,
            state.try_into_api().ok()?,
            resident,
            self.item_target(state, at.side, entity).is_some(),
        ))
    }

    fn capability_view(&self, at: BlockSide) -> Option<View> {
        if !valid_resident_position(self.world, at.pos) {
            return None;
        }
        let [x, y, z] = at.pos;
        let pos = (x, y, z);
        let state = self.world.block(x, y, z);
        let entity = self.world.block_entities.get(&pos);
        match self.item_target(state, at.side, entity)? {
            ItemTarget::Composter(level) => Some(View {
                parts: vec![(pos, 1)],
                kind: Kind::Composter(level),
                container: None,
            }),
            ItemTarget::Pot => Some(View {
                parts: vec![(pos, 1)],
                kind: Kind::Pot,
                container: None,
            }),
            ItemTarget::Container => self.view(pos, false),
        }
    }
    pub(in crate::game) fn capability_size(&self, at: BlockSide) -> Option<usize> {
        Some(self.capability_view(at)?.capability_slots(at.side).count())
    }
    pub(in crate::game) fn capability_stack(
        &self,
        at: BlockSide,
        slot: usize,
    ) -> Option<ItemStack> {
        let view = self.capability_view(at)?;
        let slot = view.capability_slots(at.side).nth(slot)?;
        self.insertion_item(&view, slot)
            .map(std::borrow::Cow::into_owned)
    }
}
impl Game<'_> {
    /// Vanilla containers and `WorldlyContainerHolder`s keep their direct automation path.
    /// This owner check borrows storage; it never builds a container view on the tick path.
    pub(super) fn vanilla_automation_container(&self, pos: Pos) -> bool {
        use sunrise_client::containers::BlockEntity;
        match self.world.block_entities.get(&pos) {
            Some(
                BlockEntity::Mod(_)
                | BlockEntity::Beehive(_)
                | BlockEntity::Campfire(_)
                | BlockEntity::Brushable(_)
                | BlockEntity::EndPortal,
            ) => false,
            Some(entity) if !entity.slots().is_empty() => true,
            _ => matches!(
                self.res
                    .facts()
                    .get(self.world.block(pos.0, pos.1, pos.2))
                    .name
                    .as_ref(),
                "minecraft:composter" | "minecraft:decorated_pot"
            ),
        }
    }
    /// `HopperBlockEntity.inventoryFull`: every slot holds exactly a full item stack.
    pub(super) fn hopper_is_stacked(&self, view: &View) -> bool {
        let c = self.containers();
        (0..view.len()).all(|slot| {
            c.get(view, slot)
                .is_some_and(|stack| stack.count == c.max_stack(stack))
        })
    }
    /// `DropperBlock.dispenseFrom`: always commit a storage attempt, including refusal.
    /// Accepted source removal precedes root-commit callbacks that can re-enter the source.
    pub(super) fn dropper_into_part(
        &mut self,
        part: &mod_api::Capability<dyn mod_api::ResourceHandler<mod_api::ItemResource>>,
        stack: &ItemStack,
        source: &View,
        slot: usize,
    ) -> bool {
        use crate::game::api_values::TryIntoApi;
        let Ok(snapshot) = stack.try_into_api() else {
            return false;
        };
        let resource = mod_api::ItemResource::of(&snapshot);
        mod_api::run_transaction(self, |game, tx| {
            let (pos, index) = source.locate(slot).ok_or(())?;
            game.journal_block_entity_slot(pos, index);
            // Reserve the offered item before writable insertion notices can re-enter.
            let reserved = game.take_one(source, slot).ok_or(())?;
            // A part answering more than it was offered is a broken mod: abort, undoing
            // whatever its insertion moved. A plain refusal still commits.
            let accepted = part.insert(game, tx, &resource, 1).map_err(|_| ())? == 1;
            if !accepted {
                match game.slot_mut_at(source, slot).ok_or(())? {
                    Some(held) if held.same_item_same_components(&reserved) => {
                        held.count = held.count.checked_add(1).ok_or(())?;
                    }
                    empty @ None => *empty = Some(reserved),
                    // A writable notice replaced the slot. Only aborting can restore the
                    // reserved item without deleting that replacement; this exceptional
                    // refusal rolls back the notice's storage writes with the attempt.
                    Some(_) => return Err(()),
                }
            }
            Ok::<_, ()>(accepted)
        })
        .unwrap_or(false)
    }
    /// The level in place (`minecraft:overworld`), for a hook that isn't told
    /// which it runs in.
    pub(in crate::game) fn dimension(&self) -> &'static str {
        self.dim.id()
    }
    /// Whether this game simulates a world, as `native_set_block` asks
    /// (the fluid helpers ask it too).
    pub(in crate::game) fn native_simulates(&self) -> bool {
        self.is_logical_server()
    }
    /// The level in place, as a handle: where the game's capability lookups (a hopper's) are.
    /// None where no world is simulated.
    pub(in crate::game) fn native_level_here(&self) -> Option<LevelRef> {
        let epoch = self.native_lifecycle.epoch()?;
        self.native_simulates()
            .then(|| self.level_ref(epoch, self.dim.key()))
    }
    /// Up to `count` out of slot `slot` of `view`, without `setChanged`.
    fn take_up_to(&mut self, view: &View, slot: usize, count: u32) -> Option<ItemStack> {
        if matches!(view.kind, Kind::Composter(_)) {
            let (x, y, z) = view.parts[0].0;
            return self.composter_capability_extract([x, y, z], count);
        }
        if view.kind == Kind::Pot {
            let pos = view.parts[0].0;
            let at = [pos.0, pos.1, pos.2];
            let mut tag = super::spill::saved_entity(&self.world, at)?;
            let mut item = tag.get("item").and_then(save::item_from_nbt)?;
            let taken = count.min(item.count);
            let out = ItemStack {
                count: taken,
                ..item.clone()
            };
            item.count -= taken;
            Self::set_pot_slot(
                self.res.items(),
                &mut tag,
                (item.count != 0).then_some(item),
            );
            super::spill::put_saved_entity(&mut self.world, at, tag);
            return Some(out);
        }
        self.containers().get(view, slot)?;
        let target = self.slot_mut_at(view, slot)?;
        let current = target.as_mut()?;
        let taken = count.min(current.count);
        let out = ItemStack {
            count: taken,
            ..current.clone()
        };
        current.count -= taken;
        if current.count == 0 {
            *target = None;
        }
        Some(out)
    }
    /// The container at `at` the item capability reaches, including the composter's
    /// state-backed ports. Inactive composter ports use its inert level-7 rules, so an input
    /// can never expose bone meal and an output can never accept compost.
    fn capability_view(&self, at: BlockSide) -> Option<View> {
        if !self.level_in_place(at.level) {
            return None;
        }
        self.containers().capability_view(at)
    }
    pub(in crate::game) fn container_set_stack_limit(
        &mut self,
        at: BlockSide,
        limit: u32,
        tx: &Transaction,
    ) -> bool {
        if !self.transaction_holds(tx) {
            return false;
        }
        if !(1..=99).contains(&limit) {
            return false;
        }
        let Some(view) = self.capability_view(at) else {
            return false;
        };
        if matches!(view.kind, Kind::Composter(_)) {
            return false;
        }
        for &(pos, _) in &view.parts {
            self.journal_block_entity(pos);
            self.world.set_container_stack_limit(pos, limit);
        }
        self.view_told(&view);
        true
    }
    /// How many slots `at`'s side reaches, including a composter's one-slot ports.
    /// None without supported storage, in a level not in place, or on a client's copy.
    pub(in crate::game) fn container_size(&self, at: BlockSide) -> Option<usize> {
        self.level_in_place(at.level).then_some(())?;
        self.containers().capability_size(at)
    }
    /// The stack in slot `slot` of those `at`'s side reaches (`container_size`).
    pub(in crate::game) fn container_stack(&self, at: BlockSide, slot: usize) -> Option<ItemStack> {
        self.level_in_place(at.level).then_some(())?;
        self.containers().capability_stack(at, slot)
    }
    /// Keeps what a capability write to slot `slot` of `view` changes: that slot alone where a
    /// write there changes nothing else, else each block entity of the view whole.
    pub(super) fn journal_view_slot(&mut self, view: &View, slot: usize) {
        match view.kind {
            Kind::Composter(_) => return,
            Kind::Pot => return self.journal_saved_pot(view.parts[0].0),
            _ => {}
        }
        let slot_only = match view.kind {
            // Plain item slots: no state beside the slot changes.
            Kind::Generic | Kind::Shulker | Kind::Mod => true,
            // State beside the slot: filling an empty input restarts a furnace's cooking; an
            // insert into an empty hopper sets its cooldown; a jukebox's disc starts its song;
            // a bookshelf's books are in its block state and last slot.
            Kind::Furnace | Kind::Hopper | Kind::Jukebox | Kind::Bookshelf => false,
            // Not shown to change only the slot (a crafter's disabled slots, a brewing stand's
            // brew), so kept whole as before.
            Kind::Crafter | Kind::Brewing => false,
            Kind::Pot | Kind::Composter(_) => unreachable!("handled above"),
        };
        if slot_only && let Some((pos, slot)) = view.locate(slot) {
            return self.journal_block_entity_slot(pos, slot);
        }
        for &(part, _) in &view.parts {
            self.journal_block_entity(part);
        }
    }
    /// Default capability writes to engine-managed containers notify once at root commit.
    fn view_told(&mut self, view: &View) {
        if matches!(view.kind, Kind::Composter(_)) {
            return;
        }
        for &(pos, _) in &view.parts {
            self.after_commit(After::Changed(pos));
        }
    }
    /// Puts as much of `stack` as slot `slot` of those `at`'s side reaches takes, as a hopper
    /// would (`canPlaceItem`, the slot's limit, the same item and components), in transaction
    /// `tx`; what didn't fit.
    pub(in crate::game) fn container_insert(
        &mut self,
        at: BlockSide,
        slot: usize,
        resource: &ItemStack,
        amount: u32,
        tx: &Transaction,
    ) -> Option<ItemStack> {
        let stack = ItemStack {
            count: amount,
            ..resource.clone()
        };
        if !self.transaction_holds(tx) {
            return Some(stack);
        }
        let Some(view) = self.capability_view(at) else {
            return Some(stack);
        };
        let Some(slot) = view.capability_slots(at.side).nth(slot) else {
            return Some(stack);
        };
        if matches!(view.kind, Kind::Composter(_)) {
            if amount == 0
                || self.containers().planning_room(&view, slot, &stack) == 0
                || !self.composter_capability_insert(at.pos, &stack)
            {
                return Some(stack);
            }
            return (amount > 1).then(|| ItemStack {
                count: amount - 1,
                ..stack
            });
        }
        let (rest, result) = self.move_in_item(
            Insertion::Capability,
            &view,
            stack,
            slot,
            &mut PlacementAttempt::default(),
        );
        // A jukebox's disc starts at the root commit (`JukeboxBlockEntity.setTheItem`).
        if result == MoveIn::Moved {
            self.view_told(&view);
        }
        rest
    }
    /// How much of `resource` the slots `at`'s side reaches hold in all (`STORAGE`'s amount),
    /// in one pass over one view of the container.
    /// None where the total does not fit in 64 bits.
    pub(in crate::game) fn container_kind_amount(
        &self,
        at: BlockSide,
        resource: &ItemStack,
    ) -> Option<u64> {
        let Some(view) = self.capability_view(at) else {
            return Some(0);
        };
        let c = self.containers();
        view.capability_slots(at.side)
            .filter_map(|slot| c.insertion_item(&view, slot))
            .filter(|held| held.same_item_same_components(resource))
            .try_fold(0u64, |total, held| total.checked_add(u64::from(held.count)))
    }
    /// Physical room through `at`'s side, subject to rules a part cannot override.
    /// This is a bound for planning, not an admission decision on an offered count.
    /// None where the total does not fit in 64 bits.
    pub(in crate::game) fn container_kind_space(
        &self,
        at: BlockSide,
        resource: &ItemStack,
    ) -> Option<u64> {
        let Some(view) = self.capability_view(at) else {
            return Some(0);
        };
        let c = self.containers();
        view.capability_slots(at.side)
            .try_fold(0u64, |total, slot| {
                total.checked_add(u64::from(c.planning_room(&view, slot, resource)))
            })
    }
    /// Visits the occupied slots `at`'s side reaches, in order, converting only reached stacks.
    pub(in crate::game) fn container_visit_entries(
        &self,
        at: BlockSide,
        visitor: &mut dyn FnMut(&ItemResource, u64) -> std::ops::ControlFlow<()>,
    ) -> Result<(), LevelReadError> {
        use crate::game::api_values::TryIntoApi;
        let Some(view) = self.capability_view(at) else {
            return Ok(());
        };
        let c = self.containers();
        for stack in view
            .capability_slots(at.side)
            .filter_map(|slot| c.insertion_item(&view, slot))
        {
            let stack = stack
                .as_ref()
                .try_into_api()
                .map_err(|_| LevelReadError::Unavailable)?;
            if visitor(&ItemResource::of(&stack), u64::from(stack.count())).is_break() {
                break;
            }
        }
        Ok(())
    }
    /// Puts up to `amount` of `resource` into the slots `at`'s side reaches, those holding it
    /// first and then the empty ones, in transaction `tx`, on one view of the container: what
    /// went in. Each slot is kept and filled as `container_insert` keeps and fills it.
    pub(in crate::game) fn container_insert_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemStack,
        amount: u64,
        tx: &Transaction,
    ) -> u64 {
        if amount == 0 || !self.transaction_holds(tx) {
            return 0;
        }
        let Some(view) = self.capability_view(at) else {
            return 0;
        };
        if matches!(view.kind, Kind::Composter(_)) {
            return u64::from(
                self.containers().capacity(&view, 0, resource) > 0
                    && self.composter_capability_insert(at.pos, resource),
            );
        }
        let (mut left, mut told) = (amount, false);
        let mut attempt = PlacementAttempt::default();
        for holding in [true, false] {
            for slot in view.capability_slots(at.side) {
                if left == 0 {
                    break;
                }
                let pick = match self.containers().insertion_item(&view, slot) {
                    Some(held) => holding && held.same_item_same_components(resource),
                    None => !holding,
                };
                if !pick {
                    continue;
                }
                let stack = ItemStack {
                    count: left.min(u64::from(u32::MAX)) as u32,
                    ..resource.clone()
                };
                let offered = stack.count;
                let (rest, result) =
                    self.move_in_item(Insertion::Capability, &view, stack, slot, &mut attempt);
                if result == MoveIn::Moved {
                    left -= u64::from(offered - rest.map_or(0, |stack| stack.count));
                    told = true;
                }
            }
        }
        if told {
            self.view_told(&view);
        }
        amount - left
    }
    /// Takes up to `amount` of `resource` out of the slots `at`'s side reaches and may take
    /// from, in transaction `tx`, on one view of the container: what came out.
    pub(in crate::game) fn container_extract_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemStack,
        amount: u64,
        tx: &Transaction,
    ) -> u64 {
        if amount == 0 || !self.transaction_holds(tx) {
            return 0;
        }
        let Some(view) = self.capability_view(at) else {
            return 0;
        };
        let (mut left, mut told) = (amount, false);
        for slot in view.capability_slots(at.side) {
            if left == 0 {
                break;
            }
            let c = self.containers();
            let Some(held) = c.insertion_item(&view, slot) else {
                continue;
            };
            if !held.same_item_same_components(resource)
                || at
                    .side
                    .is_some_and(|face| !c.can_take(&view, slot, &held, face))
            {
                continue;
            }
            self.journal_view_slot(&view, slot);
            let ask = left.min(u64::from(u32::MAX)) as u32;
            if let Some(out) = self.take_up_to(&view, slot, ask) {
                left -= u64::from(out.count);
                told = true;
            }
        }
        if told {
            self.view_told(&view);
        }
        amount - left
    }
    /// How many of `stack` slot `slot` of those `at`'s side reaches holds when empty, as
    /// NeoForge's `isValid` and `getCapacityAsLong` answer: none where it may not go in (a
    /// furnace's output, `canPlaceItem`), else the slot's limit for it. `None` asks for the
    /// general slot limit before item-specific rules.
    pub(in crate::game) fn container_capacity(
        &self,
        at: BlockSide,
        slot: usize,
        stack: Option<&ItemStack>,
    ) -> u32 {
        let Some(view) = self.capability_view(at) else {
            return 0;
        };
        view.capability_slots(at.side).nth(slot).map_or(0, |slot| {
            let containers = self.containers();
            match stack {
                Some(stack) => containers.capacity(&view, slot, stack),
                None => containers.slot_stack_limit(&view, slot, None),
            }
        })
    }
    /// Takes up to `max` out of slot `slot` of those `at`'s side reaches, in transaction `tx`;
    /// through a side, only what that side may take (`canTakeItemThroughFace`). What came out.
    pub(in crate::game) fn container_extract(
        &mut self,
        at: BlockSide,
        slot: usize,
        max: u32,
        tx: &Transaction,
    ) -> Option<ItemStack> {
        if !self.transaction_holds(tx) {
            return None;
        }
        let view = self.capability_view(at)?;
        let slot = view.capability_slots(at.side).nth(slot)?;
        let c = self.containers();
        let stack = c.insertion_item(&view, slot)?;
        if max == 0
            || at
                .side
                .is_some_and(|face| !c.can_take(&view, slot, &stack, face))
        {
            return None;
        }
        self.journal_view_slot(&view, slot);
        let out = self.take_up_to(&view, slot, max)?;
        // A jukebox's song stops at the root commit (`JukeboxBlockEntity.removeItem`).
        self.view_told(&view);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_client::containers::ModContainer;
    use sunrise_core::geometry::Direction;

    fn view(kind: Kind, slots: usize) -> View {
        View {
            parts: vec![((0, 0, 0), slots)],
            kind,
            container: (kind == Kind::Mod).then(|| {
                Arc::new(ModContainer {
                    slots,
                    ..Default::default()
                })
            }),
        }
    }

    #[test]
    fn a_face_only_narrows_the_slots_of_a_worldly_container() {
        assert_eq!(
            view(Kind::Furnace, 3).slots_to_add(Some(Direction::Up)),
            [0]
        );
        assert_eq!(
            view(Kind::Generic, 2).slots_to_add(Some(Direction::Up)),
            [0, 1]
        );
        assert_eq!(
            view(Kind::Shulker, 2).slots_to_add(Some(Direction::Up)),
            [0, 1]
        );
        assert_eq!(view(Kind::Furnace, 3).slots_to_add(None), [0, 1, 2]);
    }
}

#[cfg(test)]
#[path = "storage_attempt_tests.rs"]
mod storage_attempt_tests;

use super::*;
use crate::game::{
    Dim,
    api_values::{TryIntoApi, TryIntoEngine},
    transactions::OpenTransactions,
};
use mod_api::{
    BlockEntityAccess, BlockEntityItemError, BlockEntityRef, BlockEntityScope, BlockEntityView,
    ResidentBlockEntity, ServerHost, Transaction,
};
use std::collections::HashMap;
use sunrise_client::{
    block_entities::BlockEntityLoan, chunk_map::ChunkMap, fast_hash::FxHashMap,
    world::ModifiedChunks,
};
use sunrise_core::nbt::{Compound, JavaString, Tag};
use sunrise_items::items::Items;

#[cfg(test)]
#[path = "block_entity_tick/tests.rs"]
mod tests;

impl Game<'_> {
    /// `LevelChunk.BoundTickingBlockEntity.tick` for a resident mod block entity.
    /// Returns whether the ticker was dispatched, so skipped rows have no after-slot.
    pub(in crate::game) fn tick_mod_block_entity_loaded(&mut self, pos: Pos) -> bool {
        let Some(BlockEntity::Mod(entity)) = self.world.block_entities.get(&pos) else {
            return false;
        };
        let Some(kind) = entity.tick_kind() else {
            return false;
        };
        let Some(generation) = self.world.block_entities.generation(&pos) else {
            return false;
        };
        let live_state = self.world.block(pos.0, pos.1, pos.2);
        let block = &self.res.facts().get(live_state).name;
        if !valid_tick_subject(block, entity) {
            if let Some(id) = self
                .world
                .block_entities
                .record_invalid_mod_tick(pos, generation)
            {
                log::warn!("Block entity {id} at {pos:?} cannot tick on {block}");
            }
            return false;
        }
        let Some(level) = self.level(self.dim.id()) else {
            return false;
        };
        let subject =
            mod_api::__private::game::block_entity_ref(level, [pos.0, pos.1, pos.2], generation);
        let mut access =
            ResidentBlockEntityAccess::new(self, Some(subject), Loan::Tick(Some(live_state)));
        Self::native_block_entity_tick(kind, BlockEntityScope::new(&mut access));
        self.world
            .block_entities
            .clear_ticker_warning(pos, generation);
        true
    }

    /// Dispatch the one resident server ticker; the scope owns access to the Host.
    pub(in crate::game) fn native_block_entity_tick(
        kind: mod_api::BlockEntityTypeKey,
        subject: mod_api::BlockEntityScope<'_>,
    ) {
        Self::fire_block_entity_tick(kind, subject);
    }
}

fn valid_tick_subject(block: &str, entity: &EngineBlockEntity) -> bool {
    sunrise_client::block_entity_types::valid(block, entity.id()) == Some(true)
}

#[derive(Clone, Copy)]
enum Loan {
    // Reuse the ticker's validity result for this instance and state.
    Tick(Option<BlockStateId>),
    Menu(BlockStateId),
    OnLoad,
}

pub(in crate::game) struct ResidentBlockEntityAccess<'g, 'w> {
    game: &'g mut Game<'w>,
    subject: Option<BlockEntityRef>,
    loan: Loan,
    items_changed: bool,
}

impl<'g, 'w> ResidentBlockEntityAccess<'g, 'w> {
    fn new(game: &'g mut Game<'w>, subject: Option<BlockEntityRef>, loan: Loan) -> Self {
        Self {
            game,
            subject,
            loan,
            items_changed: false,
        }
    }

    pub(in crate::game) fn for_handle(
        game: &'g mut Game<'w>,
        subject: BlockEntityRef,
        state: BlockStateId,
    ) -> Self {
        Self::new(game, Some(subject), Loan::Menu(state))
    }

    pub(in crate::game) fn blockless(game: &'g mut Game<'w>) -> Self {
        Self::new(game, None, Loan::OnLoad)
    }

    pub(super) fn on_load(game: &'g mut Game<'w>, subject: BlockEntityRef) -> Self {
        Self::new(game, Some(subject), Loan::OnLoad)
    }

    pub(in crate::game) fn items_changed(&self) -> bool {
        self.items_changed
    }
}

impl Game<'_> {
    /// The resident mod block entity and admitted block state behind a menu callback.
    pub(in crate::game) fn resident_block_entity_target(
        &self,
        pos: Pos,
    ) -> Option<(BlockEntityRef, BlockStateId)> {
        let level = self.level(self.dim.id())?;
        let handle = self.block_entity(level, [pos.0, pos.1, pos.2])?;
        matches!(
            self.world.block_entities.get(&pos),
            Some(BlockEntity::Mod(_))
        )
        .then(|| (handle, self.world.block(pos.0, pos.1, pos.2)))
    }
}

impl BlockEntityAccess for ResidentBlockEntityAccess<'_, '_> {
    fn with_view(&mut self, call: &mut dyn FnMut(BlockEntityView<'_>)) {
        use mod_api::__private::game::Fields;
        let Some(subject) = self.subject else { return };
        let target = subject.fields();
        let [x, y, z] = target.position;
        if !self.game.level_in_place(target.level) || !self.game.world.loaded(x, z) {
            return;
        }
        let live_state = self.game.world.block(x, y, z);
        if matches!(self.loan, Loan::Menu(admitted) if admitted != live_state) {
            return;
        }
        let Ok(state) = live_state.try_into_api() else {
            return;
        };
        let slot_count = self
            .game
            .world
            .block_entities
            .mod_container(live_state.0)
            .map_or(0, |c| c.slots);
        let pos = (x, y, z);
        let Some(loan) = self
            .game
            .world
            .block_entities
            .loan_mod_entity(pos, target.generation)
        else {
            return;
        };
        if let Loan::Tick(checked) = &mut self.loan
            && *checked != Some(live_state)
        {
            if !valid_tick_subject(&self.game.res.facts().get(live_state).name, loan.entity) {
                return;
            }
            *checked = Some(live_state);
        }
        let mut data = ServerBlockEntityData {
            dim: self.game.dim,
            pos,
            loan,
            slot_count,
            items: self.game.res.items(),
            journal: &mut self.game.transactions,
            chunks: &self.game.world.chunks,
            names: &self.game.world.block_names,
            hoppers: &self.game.hoppers,
            menu: matches!(self.loan, Loan::Menu(_))
                .then_some((&mut self.game.world.modified, &mut self.items_changed)),
            dirty: false,
        };
        call(BlockEntityView::new(subject, state, &mut data));
    }

    fn host(&mut self) -> &mut dyn ServerHost {
        self.game
    }

    fn transaction(
        &mut self,
        call: &mut dyn FnMut(&mut dyn BlockEntityAccess, &Transaction) -> bool,
    ) {
        let (subject, loan) = (self.subject, self.loan);
        let committed = mod_api::run_transaction(self.game, |game, tx| {
            let mut child = ResidentBlockEntityAccess::new(game, subject, loan);
            if call(&mut child, tx) {
                Ok(child.items_changed)
            } else {
                Err(())
            }
        });
        if let Ok(changed) = committed {
            self.items_changed |= changed;
        }
    }
}

struct ServerBlockEntityData<'a> {
    pub(super) dim: Dim,
    pub(super) pos: Pos,
    pub(super) loan: BlockEntityLoan<'a>,
    pub(super) slot_count: usize,
    pub(super) items: &'a Items,
    pub(super) journal: &'a mut OpenTransactions,
    pub(super) chunks: &'a ChunkMap,
    pub(super) names: &'a FxHashMap<Pos, Tag>,
    pub(super) hoppers: &'a HashMap<Pos, super::super::machines::HopperState>,
    menu: Option<(&'a mut ModifiedChunks, &'a mut bool)>,
    dirty: bool,
}

impl ServerBlockEntityData<'_> {
    fn entity(&self) -> &EngineBlockEntity {
        self.loan.entity
    }

    fn edit(&mut self, saved: bool) -> &mut EngineBlockEntity {
        self.dirty |= saved;
        self.journal.journal_block_entity_loan(
            self.dim,
            self.pos,
            &self.loan,
            self.chunks,
            self.names.get(&self.pos),
            self.hoppers.get(&self.pos).copied(),
        );
        self.loan.entity
    }
}

impl Drop for ServerBlockEntityData<'_> {
    fn drop(&mut self) {
        if self.dirty
            && let Some((modified, _)) = &mut self.menu
        {
            let (x, _, z) = self.pos;
            self.journal
                .mark_scoped_menu_write(self.dim, (x >> 4, z >> 4), modified);
        }
    }
}

impl ResidentBlockEntity for ServerBlockEntityData<'_> {
    fn id(&self) -> &str {
        self.entity().id()
    }
    fn data_slots(&self) -> &[i32] {
        &self.entity().data
    }

    fn set_data_slot(&mut self, slot: u32, value: i32) -> bool {
        let index = slot as usize;
        if self
            .entity()
            .data
            .get(index)
            .is_none_or(|old| *old == value)
        {
            return false;
        }
        self.edit(false).data[index] = value;
        true
    }

    fn set_data_slots(&mut self, data: &[i32]) -> bool {
        if self.entity().data == data {
            return false;
        }
        let dest = &mut self.edit(false).data;
        dest.clear();
        dest.extend_from_slice(data);
        true
    }

    fn nbt(&self) -> &Compound {
        &self.entity().nbt
    }

    fn set_nbt(&mut self, key: JavaString, value: Option<Tag>) -> bool {
        let is_items = key.0.iter().copied().eq("Items".encode_utf16());
        if ["id", "x", "y", "z", sunrise_client::containers::SHAPES_KEY]
            .iter()
            .any(|reserved| key.0.iter().copied().eq(reserved.encode_utf16()))
            || (is_items && self.slot_count > 0)
            || value.as_ref().is_some_and(|tag| {
                matches!(tag, Tag::End) || sunrise_client::nbt_path::too_deep(tag, 1)
            })
        {
            return false;
        }
        let unchanged = if is_items && !self.entity().items.is_empty() {
            sunrise_client::save::block_entity_encode::mod_items(self.entity(), self.items) == value
        } else {
            self.entity().nbt.get(&key) == value.as_ref()
        };
        if unchanged {
            return false;
        }
        let entity = self.edit(true);
        if is_items {
            entity.items.clear();
        }
        match value {
            Some(tag) => {
                entity.nbt.insert(key, tag);
            }
            None => {
                entity.nbt.remove(&key);
            }
        }
        true
    }

    fn item_slots(&self) -> usize {
        self.slot_count
    }

    fn item_slot(&self, slot: u32) -> Option<(&str, u32)> {
        let index = slot as usize;
        if index >= self.slot_count {
            return None;
        }
        let stack = self.entity().items.get(index)?.as_ref()?;
        Some((&stack.item, stack.count))
    }

    fn copy_item_slot(&self, slot: u32) -> Option<mod_api::ItemStack> {
        self.entity()
            .items
            .get(slot as usize)?
            .as_ref()?
            .try_into_api()
            .ok()
    }

    fn set_item_slot(
        &mut self,
        slot: u32,
        stack: Option<mod_api::ItemStack>,
    ) -> Result<bool, BlockEntityItemError> {
        let index = slot as usize;
        let slots = self.slot_count;
        if index >= slots {
            return Err(BlockEntityItemError::SlotOutOfBounds {
                slot,
                slots: slots as u32,
            });
        }
        let stack = stack
            .as_ref()
            .map(|stack| stack.try_into_engine(self.items))
            .transpose()?;
        let entity = self.entity();
        if entity.items.get(index).and_then(Option::as_ref) == stack.as_ref()
            && !entity.has_opaque_item_slot(index)
        {
            return Ok(false);
        }
        if let Some((_, changed)) = &mut self.menu {
            **changed = true;
        }
        let entity = self.edit(true);
        if index >= entity.items.len() {
            entity.items.resize(slots, None);
        }
        if let Some(dest) = entity.item_slot_mut(index) {
            *dest = stack;
        }
        Ok(true)
    }

    fn set_client_update_fields(&mut self, fields: Compound) {
        if !fields.is_empty() || self.entity().has_client_update_fields() {
            self.edit(false).set_client_update_fields(fields);
        }
    }
}

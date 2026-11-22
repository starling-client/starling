//! Server-side `menu_slot_take`: the slot has delivered its stack and the click is paused. The
//! listener receives sequential live access (`BlockEntityScope`) to the resident backing entity.
use super::api_values::TryIntoApi;
use super::native_hooks::ResidentBlockEntityAccess;
use super::{Game, transactions::After};
use sunrise_client::{block_entities::Pos, containers::BlockEntity, menu::SlotTake};

impl Game<'_> {
    /// Deliver a take made by the current click (`Slot.onTake`), lending live resident storage.
    /// Returns `false` when a listener panicked (contained and disabled for `menu_slot_take` for
    /// the rest of the session) so an enclosing click transaction can abort.
    pub(super) fn mod_menu_slot_take(
        &mut self,
        pos: Pos,
        player: u128,
        menu: &str,
        (slot, taken, left): &SlotTake,
    ) -> bool {
        let Some(kind) = self
            .world
            .block_entities
            .get(&pos)
            .and_then(|entity| match entity {
                BlockEntity::Mod(m)
                    if mod_api::__private::block_entity_type_listens(
                        mod_api::HookId::MenuSlotTake,
                        m.id(),
                    ) =>
                {
                    mod_api::__private::block_entity_type_key(m.id())
                }
                _ => None,
            })
        else {
            return true;
        };
        let (Ok(taken), Ok(left)) = (taken.try_into_api(), left.try_into_api()) else {
            return true;
        };
        let Some((handle, state)) = self.resident_block_entity_target(pos) else {
            return true;
        };
        let failures = mod_api::runtime::isolated_failures();
        let items_changed = {
            let mut access = ResidentBlockEntityAccess::for_handle(self, handle, state);
            let scope = mod_api::BlockEntityScope::new(&mut access);
            Game::native_menu_slot_take(kind, scope, player, menu, *slot, &taken, left.as_ref());
            access.items_changed()
        };
        if items_changed && self.native_block_entity_write_key(handle) == Some(pos) {
            self.after_commit(After::Changed(pos));
        }
        mod_api::runtime::isolated_failures() == failures
    }
}

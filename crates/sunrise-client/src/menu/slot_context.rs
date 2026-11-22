//! The live slot view lent to a `menu_slot_rules` ask.
use super::{
    CONTAINER_MAX, Env, GRID, ItemStack, MAIN, Menu, PLAYER, RESULT, SlotContext, SlotRules,
};
use mod_api::MenuContainer;

/// The API's key for one of a menu's containers.
fn key(container: usize) -> MenuContainer {
    match container {
        MAIN => MenuContainer::Main,
        GRID => MenuContainer::Grid,
        RESULT => MenuContainer::Result,
        _ => MenuContainer::Player,
    }
}

/// The container a LAN view's slot `i` is in: the block's container's slots are `Main`, the
/// rest the player's.
pub fn lan_container(i: usize, container: Option<&[Option<ItemStack>]>) -> MenuContainer {
    match container {
        Some(c) if i < c.len() => MenuContainer::Main,
        _ => MenuContainer::Player,
    }
}

impl<'a> SlotContext<'a> {
    /// A blockless menu's live backing and player slots, borrowed for one slot-policy question.
    pub fn blockless(
        guest: &'a crate::lan::guest::Guest,
        pos: Option<[i32; 3]>,
        container: MenuContainer,
    ) -> Self {
        Self {
            player: Some(guest.uuid),
            pos,
            container,
            slots: &guest.native_menu,
            main_crafting: None,
            grid: &[],
            result: &[],
            player_slots: &guest.inventory,
            player_armor: Some(&guest.armor),
            player_offhand: Some(&guest.offhand),
            staged_slot: None,
        }
    }

    /// What slot `index` of `container` holds, keyed as `MenuSlotContext::slot`.
    pub fn slot(&self, container: MenuContainer, index: usize) -> Option<&ItemStack> {
        if let Some((staged_container, staged_index, stack)) = self.staged_slot
            && (container, index) == (staged_container, staged_index)
        {
            return stack;
        }
        match container {
            MenuContainer::Main => {
                if self.main_crafting.is_some_and(|crafting| {
                    crafting.grid_slots().contains(&index) || index == crafting.result
                }) {
                    None
                } else {
                    self.slots.get(index).and_then(Option::as_ref)
                }
            }
            MenuContainer::Grid => self.grid.get(index).and_then(Option::as_ref),
            MenuContainer::Result => self.result.get(index).and_then(Option::as_ref),
            MenuContainer::Player => {
                if let Some(armor) = self.player_armor
                    && (36..40).contains(&index)
                {
                    armor[39 - index].as_ref()
                } else if let Some(offhand) = self.player_offhand
                    && index == 40
                {
                    offhand.as_ref()
                } else {
                    self.player_slots.get(index).and_then(Option::as_ref)
                }
            }
            _ => None,
        }
    }
}

impl Menu {
    /// Who and where this menu's `menu_slot_rules` ask about menu slot `s` is: the env's
    /// player, the menu's block (`set_block_pos`), `s`'s container and the menu's and player's
    /// containers.
    pub(super) fn slot_context<'s>(&'s self, env: &Env, s: usize) -> SlotContext<'s> {
        let container = self
            .state
            .slots
            .get(s)
            .map_or(PLAYER, |slot| slot.container);
        SlotContext {
            player: env.player,
            pos: self.state.block_pos,
            container: key(container),
            slots: &self.state.containers[MAIN],
            main_crafting: None,
            grid: &self.state.containers[GRID],
            result: &self.state.containers[RESULT],
            player_slots: &self.state.containers[PLAYER],
            player_armor: None,
            player_offhand: None,
            staged_slot: None,
        }
    }
}

/// Apply the scoped rule to a slot's own answers (`may_place`, `may_pickup` and the maximum, in
/// that order), then enforce its item cap. The rules are asked directly at each live slot check,
/// only for a menu type a part answers; with none lent the slot's answers stand.
pub fn apply_menu_slot_rules(
    rules: Option<&dyn SlotRules>,
    menu: &str,
    slot: usize,
    stack: &ItemStack,
    context: SlotContext<'_>,
    own: (bool, bool, u32),
    item_max: u32,
) -> (bool, bool, u32) {
    let (mut may_place, mut may_pickup, mut max_stack) = own;
    if let Some(rules) = rules.filter(|rules| rules.listens(menu)) {
        rules.rule(
            menu,
            slot,
            stack,
            context,
            &mut may_place,
            &mut may_pickup,
            &mut max_stack,
        );
    }
    let max_stack = max_stack.min(CONTAINER_MAX).min(item_max);
    (may_place, may_pickup, max_stack)
}

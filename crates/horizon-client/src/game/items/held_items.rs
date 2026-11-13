//! The player's hands and what it wears: the stack in the acting hand (`getItemInHand`), the hand of
//! the item in use and running code with the other hand acting, `Inventory.add`, and the worn glider
//! and freeze-immune gear.
use super::*;

impl Game<'_> {
    pub(super) fn held(&self) -> Option<&ItemStack> {
        self.stack_in(self.held_slot())
    }

    pub(super) fn held_mut(&mut self) -> &mut Option<ItemStack> {
        self.stack_in_mut(self.held_slot())
    }

    pub(super) fn glider_worn(&self) -> bool {
        let items = self.res.items();
        let hand = |stack: Option<&ItemStack>, slot: &str| {
            stack.is_some_and(|s| horizon_client::item_sync::can_glide_using(s, slot, items))
        };
        gliding_pieces(items, &self.armor).contains(&true)
            || hand(self.inventory[self.selected].as_ref(), "mainhand")
            || hand(self.offhand.as_ref(), "offhand")
    }

    pub(super) fn wear_glider(&mut self, points: u32) {
        let gliding = gliding_pieces(self.res.items(), &self.armor);
        let pieces: Vec<usize> = (0..4).filter(|&i| gliding[i]).collect();
        if pieces.is_empty() {
            return;
        }
        // `Util.getRandom(slotsWithGliders, this.random)`: the entity's own
        // random, which never touches the level's stream.
        let pick = random_source::entity().next_int_bound(pieces.len() as i32);
        let pick = pieces[pick.unwrap_or(0) as usize];
        if let Some(stack) = self.armor[pick].clone() {
            let loss = self.wear(&stack, points);
            if let Some(piece) = self.armor[pick].as_mut() {
                piece.damage += loss;
            }
        }
    }

    pub(crate) fn wears_freeze_immune(&self) -> bool {
        let immune = self
            .res
            .items()
            .item_tags
            .get("minecraft:freeze_immune_wearables");
        self.armor
            .iter()
            .flatten()
            .any(|s| immune.is_some_and(|t| t.contains(&*s.item)))
    }

    /// `PowderSnowBlock.canEntityWalkOnPowderSnow`'s boots half: leather boots
    /// on the feet.
    pub(crate) fn wears_leather_boots(&self) -> bool {
        self.armor[3]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:leather_boots")
    }

    pub(super) fn held_slot(&self) -> usize {
        match self.hand {
            Hand::Main => self.selected,
            Hand::Off => shield::OFFHAND_SLOT,
        }
    }

    /// `LivingEntity.getUsedItemHand`: the hand of the item in use.
    pub(super) fn used_hand(&self) -> Option<Hand> {
        item_use::hand_of(self.using)
    }

    /// Run `f` with `hand` acting (`held`, `consume_held`, `swing_arm`).
    pub(super) fn with_hand<T>(&mut self, hand: Hand, f: impl FnOnce(&mut Self) -> T) -> T {
        let prior = std::mem::replace(&mut self.hand, hand);
        let out = f(self);
        self.hand = prior;
        out
    }

    /// Creative mode only: gives the player `count` of `item`
    /// (`minecraft:diamond`) as taking it from the creative inventory does
    /// (`ServerboundSetCreativeModeSlot`): the selected slot first, then the
    /// hotbar, then the inventory. The count is cut to one stack.
    pub(in crate::game) fn give(
        &mut self,
        item: mod_api::ItemKey,
        count: u32,
    ) -> Result<(), String> {
        let item = mod_api::__private::item_key_id(item).ok_or("unknown item key")?;
        self.recipe_give(item, count)
    }
}

/// The item of the stack in `selected` (`Player.tick`'s main-hand item), borrowed: a guest's
/// tick compares it with the last one and clones it only when it changed.
pub(super) fn held_attack_item(
    inventory: &[Option<ItemStack>],
    selected: usize,
) -> Option<&Arc<str>> {
    inventory.get(selected)?.as_ref().map(|s| &s.item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str) -> ItemStack {
        ItemStack {
            item: item.into(),
            count: 1,
            damage: 0,
            extra: None,
        }
    }

    #[test]
    fn held_attack_item_matches_the_cloned_stack() {
        let mut inventory: Vec<Option<ItemStack>> = vec![None; 36];
        inventory[0] = Some(stack("minecraft:stone"));
        inventory[3] = Some(stack("minecraft:diamond_sword"));
        for selected in [0, 1, 3, 35, 36, 1000] {
            // The clone the tick used to make, as the reference.
            let cloned = inventory.get(selected).cloned().flatten().map(|s| s.item);
            assert_eq!(held_attack_item(&inventory, selected).cloned(), cloned);
        }
        assert_eq!(held_attack_item(&[], 0), None);
    }
}

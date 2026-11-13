use super::*;

impl Game<'_> {
    pub(in crate::game) fn use_item_in_hand(&mut self, stack: &ItemStack, claimed: &mut bool) {
        // Starling: the old hook first, for the mods made for it (native_compat.rs).
        self.native_use_item_old(stack, claimed);
        if *claimed {
            return;
        }
        if mod_api::__private::has_part_hook(mod_api::HookId::UseItem)
            && let Ok(snapshot) = stack.try_into_api()
        {
            self.native_use_item(mod_api::__private::UseItemCall {
                stack: &snapshot,
                hand: self.hand,
                claimed,
            });
        }
    }
}

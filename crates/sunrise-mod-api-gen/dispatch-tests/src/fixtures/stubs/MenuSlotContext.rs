use super::ItemStack;

pub(super) struct MenuSlotContext<'a> {
    pub player: Option<u128>,
    pub pos: Option<[i32; 3]>,
    read: &'a dyn Fn(MenuContainer, usize) -> Option<ItemStack>,
}

impl<'a> MenuSlotContext<'a> {
    pub fn new(
        player: Option<u128>,
        pos: Option<[i32; 3]>,
        read: &'a dyn Fn(MenuContainer, usize) -> Option<ItemStack>,
    ) -> Self {
        Self { player, pos, read }
    }

    pub fn slot(&self, container: MenuContainer, index: usize) -> Option<ItemStack> {
        (self.read)(container, index)
    }
}

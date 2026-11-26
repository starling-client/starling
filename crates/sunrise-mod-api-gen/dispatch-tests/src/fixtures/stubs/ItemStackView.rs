pub(super) struct ItemStackView<'a>(pub(super) &'a super::ItemStack);

impl ItemStackView<'_> {
    pub(super) fn count(&self) -> u32 {
        self.0.count()
    }

    pub(super) fn item(&self) -> super::ItemKey {
        self.0.item()
    }
}

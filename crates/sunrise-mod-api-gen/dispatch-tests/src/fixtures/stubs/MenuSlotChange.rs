#[derive(Clone, Default, PartialEq)]
pub(super) struct MenuSlotChange {
    pub container: super::MenuContainer,
    pub slot: usize,
    pub before: Option<super::ItemStack>,
    pub after: Option<super::ItemStack>,
}

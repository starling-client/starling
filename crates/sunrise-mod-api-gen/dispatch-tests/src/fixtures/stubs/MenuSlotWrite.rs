#[derive(Clone, Default, PartialEq)]
pub(super) struct MenuSlotWrite {
    pub container: super::MenuContainer,
    pub slot: usize,
    pub stack: Option<super::ItemStack>,
}

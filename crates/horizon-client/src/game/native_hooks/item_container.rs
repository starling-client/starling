use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Item `item` (`mod_id:name`) holds items (`Capabilities.Item.ITEM`): set
    /// `slots` when the item is yours, for a backpack, a storage cell or a
    /// toolbox. Clamped to 1..=256, `ItemContainerContents`' limit; 0 leaves
    /// the item without slots, and vanilla's items have none. Asked at each
    /// call of the item container helpers (`item_container_slots`,
    /// `item_container_insert`, `item_container_extract`, in
    /// `native_item_container.rs`). The contents are kept in the stack's
    /// `minecraft:container`, as a shulker box item keeps them.
    /// API 370.
    pub(in crate::game) fn native_item_container(&self, item: &str, slots: &mut Option<u32>) {
        self.fire_item_container(item, slots);
    }
}

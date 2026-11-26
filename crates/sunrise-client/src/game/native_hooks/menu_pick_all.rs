use super::*;

impl Game<'_> {
    /// Whether a double click's gather may take a mod menu type's slot: `menu` is the menu type,
    /// `slot` the backing index in `container`, `carried` the cursor's stack and `stack` the
    /// slot's. `allowed` arrives true; the menu type's parts may refuse. A read-only fire queried
    /// directly from the gather (`AbstractContainerMenu.canTakeItemForPickAll`).
    #[inline(never)]
    pub(in crate::game) fn native_menu_pick_all(
        &self,
        call: mod_api::__private::MenuPickAllCall<'_>,
    ) {
        self.fire_menu_pick_all(call);
    }
}

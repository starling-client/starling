//! The `menu_slot_rules` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0131

pub trait Hooks {
    /// Slot placement and stack limits for registered vanilla and mod menus;
    /// mounts use `horizon:mount_inventory`, while player inventory and the
    /// creative picker have no registered type. `slot` is the backing index,
    /// including player inventory slots. The slot's own answers arrive first;
    /// the maximum is bounded to 1–99 and the item limit. Refusal also prevents
    /// quick-move top-ups. Runs in the restricted context on server and client
    /// prediction; answer from the stack and context alone, and fast.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "menu",
        key = "menu",
        context = "loop",
        cancellable = false,
        version = 2,
        target = "horizon_client_lib::menu::native_menu_slot_rules",
        neoforge = "none (vanilla Slot.mayPlace and mayPickup; NeoForge adds no event or extension for it)"
    )]
    fn menu_slot_rules();
}

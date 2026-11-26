//! The `menu_pick_all` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a double click's gather ("pick all", `PICKUP_ALL`) may take from a slot of a
    /// registered mod menu type: vanilla's `AbstractContainerMenu.canTakeItemForPickAll(carried,
    /// slot)`, which a Java menu overrides to keep a locked slot, a result slot or any slot its
    /// rules name out of the gather. Asked for each slot the gather would take from, after the
    /// game's own checks passed (the slot holds an item the cursor can take, the slot's pickup
    /// rule allows it, and the menu's own `canTakeItemForPickAll` allows it), so `allowed`
    /// arrives true. Set it false to skip that slot: the gather leaves its stack where it is and
    /// goes on to the next slot. A Part's refusal stands; a later Part may not turn it back.
    ///
    /// `menu` is the menu type, `slot` the backing index in `container` ([`MenuContainer`],
    /// as `menu_slot_rules` names it: `Player` for the player's inventory slots shown in the
    /// menu), `carried` the cursor's stack being gathered into and `stack` the slot's stack.
    /// `context` lends the player, the menu's block position and the live slots.
    ///
    /// Runs on the server and on the client's prediction with the read `Host`: a query, so it
    /// can neither write nor cancel. Vanilla menu types keep their own answer and never ask. A
    /// client without this mod predicts vanilla here; the server corrects it. A panicking
    /// Part's answer is discarded and its callback disabled for the session; the other Parts
    /// still answer. Answer quickly from the question and the read-only Host.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "both",
        contract = "listen",
        scope = "menu",
        key = "menu",
        host = "read",
        cancellable = false,
        version = 1,
        neoforge = "none (vanilla AbstractContainerMenu.canTakeItemForPickAll, which each menu overrides)"
    )]
    fn menu_pick_all(
        menu: &str,
        slot: usize,
        container: MenuContainer,
        carried: &ItemStack,
        stack: &ItemStack,
        context: &MenuSlotContext<'_>,
        allowed: &mut bool,
    );
}

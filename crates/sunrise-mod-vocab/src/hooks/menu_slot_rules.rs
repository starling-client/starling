//! The `menu_slot_rules` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Slot placement, pickup and stack limits for registered vanilla and mod
    /// menus; mounts use `sunrise:mount_inventory` and the player's own
    /// `InventoryMenu` `minecraft:inventory` (vanilla gives it no `MenuType`),
    /// while the creative picker has no registered type. The inventory has no
    /// owner: every mod's Parts attached to it are asked, in mod order, and a
    /// refusal from any of them stands. Its `container` is `Result` for menu
    /// slot 0, `Grid` for the 2×2 grid (slots 1 to 4, row by row) and `Player`
    /// for the rest. `slot` is the backing index,
    /// including player inventory slots. The slot's own answers arrive first;
    /// the maximum is bounded to 0–99 and the item limit (matching vanilla
    /// `Slot.getMaxStackSize` and `Container.getMaxStackSize`). Refusal also
    /// prevents quick-move top-ups. Runs on server and client prediction with
    /// the read `Host`: it is a query, so it can neither write nor cancel. Each
    /// live `may_place`, `may_pickup` and `getMaxStackSize` check during a click
    /// asks the parts directly against the current menu slots (matching
    /// vanilla `AbstractContainerMenu.doClick` and `Slot`). Permission and
    /// capacity are separate checks. During shift-click a later destination
    /// sees the source remainder and the earlier destinations' updates.
    /// A nested `MenuRef::slot_holds` or `MenuRef::slot_gives` read uses the
    /// slot's own rules without asking menu Parts again, even for another slot
    /// or menu. `context.slot` still reads the slots of the ongoing click.
    /// A panicking part's answers are discarded and its callback is disabled
    /// for the session, with the mod named in the log and error toast. The
    /// other parts still answer the check.
    /// Answer quickly from the question and the read-only Host. On a LAN
    /// host's own click the LAN host is not reachable through the `Host`
    /// (it answers as if no LAN were open).
    /// `container` is the container `slot` indexes ([`MenuContainer`]): a mod
    /// crafting grid's slots are `Grid`'s and its result `Result`'s slot 0.
    /// `context` lends the player, the menu's block position and the live
    /// slots for this ask ([`MenuSlotContext`]). Read slots on demand with
    /// `context.slot(container, index)`, covering both the menu's backing
    /// containers (`Main`, `Grid`, `Result`) and the player's inventory
    /// (`Player`: `0..36` inventory, `36..40` armor feet-to-head, `40` offhand,
    /// matching vanilla `Inventory`). Bound linked slots read their actual storage; a
    /// prepared crafting result belongs to the grid that was shown with it.
    /// A client without this mod predicts vanilla here; the server corrects it.
    ///
    /// A stack that cannot cross to the API (an item with no API key, count 0, damage out of
    /// range) is never offered to the parts: placing it into a slot of a menu type a part
    /// answers is refused, picking it up keeps the slot's own answer, and the game logs a
    /// warning.
    ///
    /// `may_pickup` is `Slot.mayPickup`, answered only for a registered mod
    /// menu type (its own slots and the player's inventory slots in it) and
    /// for the player's own inventory menu; another vanilla menu type keeps its
    /// slots' own answer. It is asked for the
    /// stack the slot holds, wherever the click would take from it: a pickup
    /// click on a slot that holds an item (after `item_stacked_on_other`,
    /// before any take, merge or exchange), quick move (once, on the clicked
    /// slot, before the move; moving into a slot asks only `may_place`), a
    /// hotbar or offhand swap that takes the slot's item (with both stacks
    /// present `may_place` must hold too), throw (inside the take, so a refusal
    /// drops nothing and stops the repeat), pick-all (it gathers when the
    /// clicked slot is empty or refuses, and skips each source that refuses),
    /// and every take the menu makes. A drag spread, a creative clone, an
    /// insertion into an empty slot and a click outside the window never ask
    /// it; a drag that ends on one slot is a pickup click on it. A refusal
    /// leaves the slot and the cursor as they were (nothing is taken, swapped
    /// or merged) and the server corrects the client's prediction. A slot that
    /// refuses placement gives all of its stack or none. A display-only slot
    /// answers `may_pickup` false and `may_place` false. `may_pickup` arrives
    /// true: the slot's own rule comes first, and a slot it already refuses is
    /// not asked. A shift-click a part answers with `menu_quick_move` is
    /// asked the pickup rule before that hook.
    #[hook(
        receiver = "read",
        dispatch = "api_carrier",
        side = "both",
        contract = "listen",
        scope = "menu",
        key = "menu",
        host = "read",
        cancellable = false,
        version = 9,
        neoforge = "none (vanilla Slot.mayPlace and mayPickup; NeoForge adds no event or extension for it)"
    )]
    fn menu_slot_rules(
        menu: &str,
        slot: usize,
        container: MenuContainer,
        stack: &ItemStack,
        context: &MenuSlotContext<'_>,
        may_place: &mut bool,
        may_pickup: &mut bool,
        max_stack: &mut u32,
    );
}

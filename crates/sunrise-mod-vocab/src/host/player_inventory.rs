//! The `player_inventory` area: a player's inventory, ender chest and selected slot. Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_inventory` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerInventoryBackend {
    /// Side: both.
    /// `Inventory.getSelectedSlot`: the hotbar slot the player holds, 0..9. On a client,
    /// [`crate::ClientHost::select_hotbar_slot`] chooses it.
    fn player_selected_slot(&self, player: PlayerRef) -> Option<usize>;
    /// Side: both.
    /// Lend the inventory once if available, including when empty.
    /// See [`PlayerRef::with_inventory`] for the slot numbering and side rules.
    #[manual_handle]
    fn player_inventory_slots(
        &self,
        player: PlayerRef,
        read: &mut dyn for<'a> FnMut(InventoryView<'a>),
    ) -> Option<()>;
    /// Side: server.
    /// Lend the ender chest once if available, including when empty.
    /// See [`PlayerRef::with_ender_chest`] for the slot numbering and side rules.
    #[manual_handle]
    fn player_ender_chest(
        &self,
        player: PlayerRef,
        read: &mut dyn for<'a> FnMut(InventoryView<'a>),
    ) -> Option<()>;
}
/// The handles' writes of the `player_inventory` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait PlayerInventoryBackendMut {
    /// Side: server.
    /// `Inventory.setSelectedSlot`: makes `slot` (0 to 8) the hotbar slot the player holds and
    /// tells their client (`ClientboundSetHeldSlot`), as `/clear`-style server code does. A new
    /// slot ends a main-hand use in progress, as the client's own switch does, and asks no
    /// `player_switch_hotbar_slot_pre`: that hook is for the player's own input. Returns false for
    /// a slot past 8 or a stale or unavailable player; true when the player already held `slot`.
    /// A resident dead player can still be edited.
    fn player_set_selected_slot(&mut self, player: PlayerRef, slot: usize) -> bool;
    /// Side: server.
    /// `Player.drop(stack, false, thrown_by_player)`: `stack` is thrown from the player's eyes
    /// along their view as an item entity, with the main hand swinging and `item_toss` asked
    /// first. Nothing is taken from an inventory: the stack is the caller's, taken out through
    /// the item places first, which is what separates it from giving or dropping into the
    /// inventory. `thrown_by_player` names the player as the item's thrower, as the drop key
    /// does. `Ok(None)` when the stack is in the world; otherwise `Ok(Some(stack))` comes back, untouched, for
    /// the caller to put back: an empty stack, a stale or dead player, a player in another level
    /// than the hook's, or a toss a mod canceled (`item_toss`), so a refused drop never loses
    /// items. Invalid API stack conversion returns
    /// `StackWriteError`, as other checked stack writes do.
    fn player_drop_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
        thrown_by_player: bool,
    ) -> Result<Option<ItemStack>, StackWriteError>;
    /// Numbers: 0..35 main (0..8 hotbar), 36..39 feet to head, 40 offhand,
    /// 41 body, 42 saddle. ItemPlace uses 100..103 for armour and -106 for offhand;
    /// InventoryMenu has no 41 or 42.
    /// `Inventory.setItem(slot, stack)` by [`PlayerRef::with_inventory`]'s numbers: `stack` (None
    /// empties it) goes into that slot as `/item replace entity <player> <slot>` puts it,
    /// without the slot's item check, so a worn slot takes any item; the equip sound and
    /// equipment changes follow for 36 to 42. The client is sent the slot with the tick's
    /// inventory sync (`ContainerSetSlot`), as vanilla's `broadcastChanges`. Each refusal
    /// writes nothing and is named by `PlayerDataRefusal`: `Stale` (no online player behind
    /// the handle or the player's state is unavailable), `Client`, `SlotOutOfRange` (a slot
    /// past 42, never a panic) and `Stack` (an item the world lacks, a count or damage the
    /// item does not allow). A resident dead player can still be edited.
    /// An open transaction journals this write; an abort restores the previous value.
    /// Equipment effects wait for the outermost commit.
    fn player_set_inventory_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// `PlayerEnderChestContainer.setItem(slot, stack)`: `stack` (None empties it) in ender
    /// chest slot `slot` (0 to 26). A player looking into their ender chest is sent the slot
    /// with the open menu's sync. Refuses as `player_set_inventory_slot` does, with
    /// `SlotOutOfRange { last: 26 }` for a slot past 26 (never a panic). The chest is
    /// editable while the player is dead and carries over on respawn.
    /// An open transaction journals this write; an abort restores the previous value.
    fn player_set_ender_chest_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal>;
    /// Side: server.
    /// Gives `stack` as `/give` does, components and all: `Inventory.add` fills matching
    /// stacks, then empty slots (the selected one first); what does not fit is thrown from
    /// the player's eyes with no pickup delay and only they may pick it up. A stack that fits
    /// instead spawns the short-lived, non-pickable `/give` visual and plays the pickup sound.
    /// Refuses, with nothing given, with `Stale`, `Client`,
    /// `Dead` or `Stack`, with no inventory or world changes.
    /// This happens in the player's own level, even when the hook runs in another one.
    /// `Ok(None)` means every item was accepted. The ordinary entity-join hook can still
    /// cancel the overflow drop. If the player's level is unavailable, returns the overflow
    /// as `Ok(Some(rest))` rather than throwing it elsewhere.
    /// An open transaction journals this write; an abort restores the previous value.
    /// Overflow drops, the pickup visual and the sound wait for the outermost commit.
    fn player_give_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
    ) -> Result<Option<ItemStack>, PlayerDataRefusal>;
}

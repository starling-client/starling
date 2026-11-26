//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `menus` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait MenusClientHost {
    /// Side: client.
    /// The local player in a logged-in client world.
    fn local_player(&self) -> Option<PlayerRef>;
}
/// The handles' reads of the `menus` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait MenusBackend {
    /// Side: server.
    /// Container id (0 for the player's inventory menu).
    fn menu_container_id(&self, menu: MenuRef) -> Option<i32>;
    /// Side: both.
    /// Registry menu type (`AbstractContainerMenu.getType`): `Some(Some(id))` for a
    /// supported registry-backed menu (such as `minecraft:furnace`, `minecraft:generic_9x3`, or
    /// a mod's menu type); `Some(None)` for the player's inventory menu and, on a client, a mount's inventory,
    /// which have no MenuType in vanilla. The outer None means a stale, foreign or unavailable
    /// menu: a server's mount menu handles are unavailable, and on a client only the local
    /// player's supported live menu is available.
    fn menu_kind(&self, menu: MenuRef) -> Option<Option<String>>;
    /// Side: both.
    /// Number of slots in AbstractContainerMenu.slots.
    fn menu_slot_count(&self, menu: MenuRef) -> Option<usize>;
    /// Side: both.
    /// Full stack at a menu index. Outer None means unavailable or out of
    /// range; Some(None) means an empty slot. `slot` is the slot's position in the menu's slot
    /// list, vanilla's `Slot.index`, the number container click packets carry; vanilla has no other
    /// player-visible slot number. `menu_slot_backing` gives the index inside the backing container.
    fn menu_slot(&self, menu: MenuRef, slot: usize) -> Option<Option<ItemStack>>;
    /// Side: both.
    /// Carried (cursor) stack; outer None means unavailable.
    fn menu_carried(&self, menu: MenuRef) -> Option<Option<ItemStack>>;
    /// Side: both.
    /// The menu's synchronized state id (`AbstractContainerMenu.getStateId`): the server counts
    /// it up with every slot or content sync it sends for the menu, and a client holds the value
    /// of the last `ContainerSetContent` or `ContainerSetSlot` it received for it, so a fresh
    /// snapshot shows even when the stacks look the same. None for a stale, foreign or closed
    /// menu, as for `menu_carried`.
    fn menu_state_id(&self, menu: MenuRef) -> Option<i32>;
    /// Side: server.
    /// The storage menu slot `slot` shows (`Slot.container` and `getContainerSlot`): the
    /// player's inventory by `ENTITY_ITEMS` index (`InventoryMenu`'s armour slots 5..9 go head
    /// to feet), a block entity's or an entity's slot (following a mod menu's linked storage),
    /// the ender chest's, the crafting grid's or a mod's chest or transient container slot.
    /// Returns `None` for an unavailable menu, an out-of-range index, or an unbound linked slot.
    fn menu_slot_backing(&self, menu: MenuRef, slot: usize) -> Option<MenuSlotBacking>;
    /// Side: server.
    /// How many of `stack` the live menu slot holds (`Slot.mayPlace`, then `Slot.getMaxStackSize`):
    /// a furnace's fuel slot one bucket, an armour slot one piece that fits it (asking the item's
    /// `can_equip` part for this menu's player), a crafting result none. An owned transient slot
    /// uses its declared limit or the item's own limit, whichever is lower. Excluded targets
    /// (such as a backpack's locked hotbar slot) and unbound linked slots hold nothing (`Some(0)`).
    ///
    /// Returns `None` for an enabled handler-backed slot ([`MenuSlotBacking::Place`]), because
    /// querying a [`ResourceHandler`] requires server-action authority. Read-only callers can
    /// distinguish that case from a stale menu or out-of-range index with
    /// [`MenuRef::slot_backing`]: `Some(MenuSlotBacking::Place(_))` names a handler-backed slot,
    /// which can be queried from a `&mut dyn ServerHost` through [`MenuRef::probe_slot_holds`].
    fn menu_slot_holds(&self, menu: MenuRef, slot: usize, stack: &ItemStack) -> Option<u32>;
    /// Side: server.
    /// Whether the live menu slot gives up what it holds (`Slot.mayPickup`): bound armour stays
    /// on outside creative, and excluded targets or unbound linked slots refuse pickup
    /// (`Some(false)`).
    ///
    /// Returns `None` for an enabled handler-backed slot ([`MenuSlotBacking::Place`]), because
    /// simulating a pickup against a [`ResourceHandler`] requires server-action authority.
    /// Read-only callers can distinguish that case from a stale menu or out-of-range index with
    /// [`MenuRef::slot_backing`]: `Some(MenuSlotBacking::Place(_))` names a handler-backed slot,
    /// which can be probed from a `&mut dyn ServerHost` through [`MenuRef::probe_slot_gives`].
    fn menu_slot_gives(&self, menu: MenuRef, slot: usize) -> Option<bool>;
    /// Side: server.
    /// Ask menu placement rules before handing an explicit transfer to the backing handler.
    #[manual_handle]
    fn menu_slot_holds_with_stack(
        &self,
        menu: MenuRef,
        slot: usize,
        stack: &ItemStack,
    ) -> Option<u32>;
    /// Side: server.
    /// Ask `Slot.mayPickup` with the stack just read from a backed menu slot's handler.
    #[manual_handle]
    fn menu_slot_gives_with_stack(
        &self,
        menu: MenuRef,
        slot: usize,
        stack: &ItemStack,
    ) -> Option<bool>;
}
/// The handles' client reads of the `menus` area, implemented by the game and reached through [`ClientBackend`].
// hidden: a handle's backend, reached through `ClientBackend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait MenusClientBackend {
    /// Side: client.
    /// The open inventory or creative screen's slot boxes in menu slot order
    /// (`AbstractContainerScreen.leftPos`/`topPos` plus each `Slot.x`/`Slot.y`, 16x16). A box
    /// is `[x, y, width, height]` in GUI pixels, with `None` for a hidden slot.
    /// The creative screen gives the player's `InventoryMenu` (46 slots) where
    /// `CreativeModeInventoryScreen.selectTab` places them: on the inventory tab,
    /// armor `5..=8`, main `9..=35`, hotbar `36..=44` and offhand `45` have boxes and
    /// the crafting slots `0..=4` are `None`; on an item or search tab only the
    /// hotbar `36..=44` has boxes.
    /// Returns `None` for a stale menu, another screen, or an inventory covered
    /// by the recipe book. The boxes follow the actual layout, including GUI
    /// scale and the recipe book's horizontal offset.
    fn menu_slot_boxes(&self, menu: MenuRef) -> Option<Vec<Option<[f32; 4]>>>;
    /// Side: client.
    /// The creative item or search tab's catalog cells (`ItemPickerMenu`'s 45
    /// `CustomCreativeSlot`s, 9 by 5, left to right then top to bottom), which
    /// [`MenuRef::slot_boxes`] leaves out because they are not `InventoryMenu`'s slots.
    /// Each cell is its box (`[x, y, width, height]` in GUI pixels, as `slot_boxes` gives:
    /// `leftPos`/`topPos` plus `9 + column * 18`, `18 + row * 18`, 16x16) and the stack it
    /// shows from the tab's list scrolled to its first visible row (`scrollTo`: list index
    /// `first_row * 9 + cell`), components included; `None` for a cell past the list's end.
    /// `Some` of an empty list on the creative inventory tab, which shows no catalog.
    /// Returns `None` for a stale menu, another screen, or no window.
    /// Built per call; read it in the frame that draws over it.
    fn menu_creative_catalog(&self, menu: MenuRef) -> Option<Vec<([f32; 4], Option<ItemStack>)>>;
    /// Side: client.
    /// Slot roles from the client layout and content menu declaration, in menu order.
    /// The length equals `slot_count`. Machine inputs (including fuel and brewing
    /// bottle slots) are `Input`; unknown roles are `Other`. Roles describe neither
    /// server backing nor rules for accepting items. Returns `None` outside
    /// a world, without an open menu, for stale/foreign menus or missing content types.
    /// Game types with no known slot count return `None`; workstations without
    /// classified layouts give all `Other` when their count is known. Creative returns the underlying inventory menu, not tab wrappers:
    /// read the screen type to distinguish them. Stable until the menu changes.
    /// Built per call with one Vec allocation and a table lookup; cache for the menu.
    fn menu_slot_kinds(&self, menu: MenuRef) -> Option<Vec<MenuSlotKind>>;
    /// Side: client.
    /// The local player's open merchant menu's synchronized offers, copied in display order
    /// (`MerchantMenu.getOffers`). Includes both payments, the result with its components,
    /// and the synchronized stock and price data. Use [`MerchantOffer::cost_a_count`] for
    /// the adjusted first payment. Reading does not select or take a trade or send a packet.
    ///
    /// `Ok(None)` means the menu is stale, foreign, closed, or not a merchant menu.
    /// An open merchant menu without offers returns `Ok(Some(Vec::new()))`.
    /// Returns [`LevelReadError::Unavailable`] when the phase or backend cannot supply the
    /// read, or an offer cannot be converted to an API value.
    fn menu_merchant_offers(
        &self,
        menu: MenuRef,
    ) -> Result<Option<Vec<MerchantOffer>>, LevelReadError>;
}
/// The handles' client writes of the `menus` area, implemented by the game and reached through [`ClientBackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait MenusClientBackendMut {
    /// Side: client.
    /// Submit clicks on the local player's live menu through normal prediction
    /// and `ContainerClick`. Slots use the same menu order as `slot`; `None`
    /// is outside (-999). All clicks are validated before any prediction or send.
    /// A drag expands to one complete quick-craft gesture.
    /// Refuses stale menus, invalid slots/hotbar indices, spectators, non-client hosts,
    /// the creative inventory screen and empty batches without changing items.
    /// True means submitted, not accepted by the server: its corrections win.
    fn menu_click(&mut self, menu: MenuRef, clicks: &[MenuClick]) -> bool;
    /// Side: client.
    /// Creative mode only: destroys the local player's carried (cursor) stack on the creative
    /// inventory screen, as its destroy slot does (`CreativeModeInventoryScreen.slotClicked`,
    /// `ItemPickerMenu.setCarried(EMPTY)`). With no active server container, that cursor is
    /// the client's alone, so it is cleared locally and nothing is sent.
    ///
    /// `Ok(true)` means destroyed, `Ok(false)` that nothing was carried. Refuses outside a ready
    /// client world (`Unavailable`), for a stale menu (`Stale`), outside creative mode
    /// (`NotCreative`), or if the creative inventory screen is not visible or a server
    /// container is active (`ServerOwnedMenu`). Refusals are checked before the cursor is read,
    /// change nothing and send nothing.
    fn menu_destroy_carried(&mut self, menu: MenuRef) -> Result<bool, CarriedDestroyError>;
    /// Side: client.
    /// Close the local player's open container screen as Escape does
    /// (`AbstractContainerScreen.onClose`, `LocalPlayer.closeContainer`): the
    /// server is told (`ContainerClose`) and the screen goes. Refuses stale
    /// menus, non-client hosts and a menu whose screen is not open, such as
    /// the inventory menu while playing. A server closes a menu with `close_container`.
    fn menu_close(&mut self, menu: MenuRef) -> bool;
    /// Side: client.
    /// Press button `button` of the local player's open menu as a click on
    /// one of its buttons does (`MultiPlayerGameMode.handleInventoryButtonClick`,
    /// `ContainerButtonClick`): the server's menu decides what it does
    /// (`clickMenuButton`, and `menu_button` for a mod's menu). For a client
    /// mod's own input on a menu's screen other than a press, such as a key
    /// or the wheel; no sound, that is the screen's. Refuses
    /// stale menus, non-client hosts and the inventory menu, which no server
    /// opened. True means sent, not that the menu took it.
    fn menu_click_button(&mut self, menu: MenuRef, button: i32) -> bool;
}
/// The handles' server writes of the `menus` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[sunrise_mod_api_macros::host_area]
pub trait MenusBackendMut {
    /// Side: server.
    /// How many of `stack` the live menu slot holds (`Slot.mayPlace`, then `Slot.getMaxStackSize`),
    /// probing the live [`ResourceHandler`] (`is_valid`, then `capacity_int` as in NeoForge's
    /// `ResourceHandlerSlot.getMaxStackSize`) when the slot is backed by
    /// [`MenuSlotBacking::Place`], and delegating to the same live slot evaluator as
    /// [`MenuRef::slot_holds`] for every other slot.
    ///
    /// Returns `Ok(None)` when the menu is unavailable, the slot is out of range, or a bound held
    /// container no longer resolves. Returns `Err(LevelReadError::Unavailable)` when called outside
    /// a live world phase or without a server backend.
    #[forward_api_inputs]
    fn menu_probe_slot_holds(
        &mut self,
        menu: MenuRef,
        slot: usize,
        stack: &ItemStack,
    ) -> Result<Option<u32>, LevelReadError>;
    /// Side: server.
    /// Whether the live menu slot gives up what it holds (`Slot.mayPickup`), simulating one
    /// extraction in a rolled-back transaction against the live [`ResourceHandler`] (NeoForge's
    /// `ResourceHandlerSlot.mayPickup`) when the slot is backed by [`MenuSlotBacking::Place`], and
    /// delegating to the same live slot evaluator as [`MenuRef::slot_gives`] for every other slot.
    ///
    /// Returns `Ok(None)` when the menu is unavailable, the slot is out of range, or a bound held
    /// container no longer resolves. Returns `Err(LevelReadError::Unavailable)` when called outside
    /// a live world phase or without a server backend.
    fn menu_probe_slot_gives(
        &mut self,
        menu: MenuRef,
        slot: usize,
    ) -> Result<Option<bool>, LevelReadError>;
    /// Side: server.
    /// Close a player's open menu as `ServerPlayer.closeContainer` does. The server queues
    /// `ContainerClose`, then removes the menu, returns the carried stack to the inventory,
    /// and fires `player_container_close`. In the player's own turn, uses their active
    /// inventory and cursor (closing at the call inside a hook of their own item use, block use
    /// or command, as vanilla's `ServerPlayer.openMenu` closes the open menu there before it opens
    /// the next one). Refuses stale or already removed menus, clients, a player held out by
    /// another player's active turn, a pending disconnect and the player's own inventory menu.
    /// Outside a transaction the close is immediate. Inside one, true accepts a request to close
    /// that same window at outermost commit; an abort leaves it open. A client closes its own
    /// screen with `close`.
    fn menu_close_container(&mut self, menu: MenuRef) -> bool;
    /// Side: server.
    /// Set a data slot of a live blockless mod menu (`AbstractContainerMenu.setData`).
    /// Returns true only if the value changed. Publishes the final signed `i32` at
    /// root commit (immediately outside a transaction) if the index fits `u16`.
    /// Larger declared indices remain in server storage. An abort restores the
    /// value silently. Stale handles, unsupported menus and undeclared indices are
    /// refused.
    fn menu_set_data_slot(
        &mut self,
        menu: MenuRef,
        index: usize,
        value: i32,
    ) -> Result<bool, MenuDataRefusal>;
    /// Side: server.
    /// Replaces everything the open native menu `menu` shows with `items` (by slot, padded with
    /// empty ones, extra ones dropped, as [`PlayerRef::open_chest_menu`] takes them) and sends
    /// it at once as one `ClientboundContainerSetContentPacket`
    /// (`AbstractContainerMenu.sendAllDataToRemote`), without reopening the menu: the title,
    /// the cursor and the screen stay. Writing slots through `ItemPlace::MenuSlot` sends each
    /// changed slot with the next tick's sync instead; this is the whole-contents form, for a
    /// page turned or a list rebuilt. Each refusal sends nothing and changes nothing, and is
    /// named by [`MenuItemsRefusal`]: `Client`, `NoPlayer`, `NotOpen` (stale or not an open
    /// native menu), `Stack` (a stack the game's item conversion refuses), `OwnedSlots` (use
    /// `ItemPlace::MenuSlot` for an owned window), or `Unavailable` (outside the world phase).
    /// Inside a transaction, an abort puts the display contents back without sending a packet.
    fn menu_set_items(
        &mut self,
        menu: MenuRef,
        items: Vec<Option<ItemStack>>,
    ) -> Result<(), MenuItemsRefusal>;
    /// Side: server.
    /// Backs the container slots of this blockless mod menu with storage that is not a block
    /// entity (#3385; NeoForge's `StackCopySlot` and `ResourceHandlerSlot`): `places[n]` is what
    /// slot `n` shows. From then on a click or shift-click on those slots, and an
    /// `ItemPlace::MenuSlot` write, go through that storage's own handler, journalled; the client
    /// sees ordinary synced slots, mirrored after each click and each write through an
    /// `ItemPlace` of that storage, never each tick. Right after the open is the place to call
    /// it, the player's own item use included: backed inside a hook of that player's own action,
    /// the slots are mirrored once the action is over. A slot past `places` stays the mods' own
    /// (`menu_clicked`). The menu closes once a place stops resolving (the backpack left the
    /// hand, the entity is gone) or a held stack is no longer the one bound when the slots were
    /// backed, checked before each click and at each change notice. A `MenuSlotPlace::HeldItem`
    /// locks the inventory slot holding that stack while the menu is open, so it can't go into
    /// itself. Backing again replaces the places; a slot no longer backed (an empty list, a
    /// shorter one) is emptied, never left holding the copy it showed. A [`MenuSlotBackRefusal`],
    /// and nothing changes, for a stale menu or offline player, a menu that is not a blockless
    /// mod menu, a menu that declares `owned_slots`, more places than its container slots, or a
    /// place that doesn't resolve now.
    fn menu_back_slots(
        &mut self,
        menu: MenuRef,
        places: Vec<MenuSlotPlace>,
    ) -> Result<(), MenuSlotBackRefusal>;
}

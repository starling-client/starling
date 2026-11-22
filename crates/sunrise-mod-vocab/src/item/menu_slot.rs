//! A player's open menu's slots as item places (arismoko/horizon#41 §7), a fork addition beside
//! NeoForge 26.3's `ItemAccess.forPlayerSlot`. A slot is the storage it shows, so a write through
//! [`ItemPlace::MenuSlot`] is a write to that storage, journaled once, through its own handler,
//! under the slot's own rules.
use crate::{
    EntityRef, Hand, ItemPlace, LevelRef, MenuKey, MenuRef, MenuReplace, MenuSlotPlace, Text,
};

/// The complete request to open a blockless mod menu, passed to [`crate::PlayerRef::open_mod_menu`].
// no wire: A native Host request; the game sends the title, extra and data via the menu protocol.
#[non_exhaustive]
pub struct ModMenuOpen {
    /// The declared mod menu type.
    pub menu: MenuKey,
    /// The menu title sent to the client.
    pub title: Text,
    /// One-time opening bytes, at most 32 KiB. The client reads them through
    /// [`crate::MenuScreenOpen::extra`] only while its factory builds the screen.
    pub extra: Vec<u8>,
    /// Exactly the menu type's declared number of live data values, in slot order.
    pub data: Vec<i32>,
    /// Optional position reported to the screen; it supplies no block backing.
    pub pos: Option<[i32; 3]>,
    /// The hand used to open the menu from an item, or `None` for a different opener.
    pub opening_hand: Option<Hand>,
    /// Whether to send the client a close before replacing the old window.
    pub replace: MenuReplace,
}

impl ModMenuOpen {
    /// A request with no extra data, opening hand or screen position. Set `data` if the
    /// menu declares live data slots. The previous window is closed before opening this one.
    pub fn new(menu: MenuKey, title: Text) -> Self {
        Self {
            menu,
            title,
            extra: Vec::new(),
            data: Vec::new(),
            pos: None,
            opening_hand: None,
            replace: MenuReplace::CloseFirst,
        }
    }
}

/// The hand that opened a mod menu and its status at a `menu_still_valid` check.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub struct MenuOpeningItem {
    /// The hand supplied when the menu opened.
    pub hand: Hand,
    /// That hand holds an item with the same item type and components as at open. Count is
    /// ignored. False if the opening hand was empty. This compares values, not instance identity.
    pub stack_matches: bool,
}

impl MenuOpeningItem {
    // hidden: The engine constructs this event value; mods only read it.
    #[doc(hidden)]
    pub fn new(hand: Hand, stack_matches: bool) -> Self {
        Self {
            hand,
            stack_matches,
        }
    }
}

/// What a menu slot shows: where [`MenuRef::slot_backing`] says its stack lives.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum MenuSlotBacking {
    /// Index `.0` of the menu's player's [`ENTITY_ITEMS`](crate::ENTITY_ITEMS): the inventory
    /// 0..36, the armour 36..40 from feet to head, the off hand 40.
    Player(usize),
    /// Slot `slot` of the block entity at `pos`, through its [`ITEMS`](crate::ITEMS) handler.
    BlockEntity {
        /// The level the block entity is in.
        level: LevelRef,
        /// The block entity's position.
        pos: [i32; 3],
        /// The slot's index in the handler.
        slot: usize,
    },
    /// Slot `slot` of `entity`'s [`ENTITY_ITEMS`](crate::ENTITY_ITEMS): a chest minecart's, a
    /// chest boat's or a chested mount's.
    Entity {
        /// The entity holding the inventory.
        entity: EntityRef,
        /// The slot's index in the handler.
        slot: usize,
    },
    /// Slot `.0` of the player's ender chest.
    Ender(usize),
    /// Slot `.0` of the player's inventory crafting grid, 0..4.
    CraftGrid(usize),
    /// The inventory crafting grid's result, which nothing writes: a craft is a click.
    CraftResult,
    /// Slot `.0` of a mod's display menu with no declared transient or handler backing.
    Native(usize),
    /// A container slot of a mod menu type opened with no block, backed by storage that is not a
    /// block entity ([`PlayerRef::back_mod_menu_slots`](crate::PlayerRef::back_mod_menu_slots)):
    /// a held item's contents or an entity's item handler. Only such a slot has this backing.
    Place(MenuSlotPlace),
    /// Slot `.0` of this window's own transient container. It is never backed by another
    /// inventory or saved independently; its final value appears in `menu_closed`.
    Owned(usize),
}

// no wire: An in-process refusal from the native menu setter, not a transported payload.
/// Why [`MenuRef::set_data_slot`] left the menu unchanged.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuDataRefusal {
    /// This host or menu does not support blockless mod menu data.
    Unsupported,
    /// The player or the menu generation is no longer current.
    Stale,
    /// The caller does not own the menu type.
    WrongOwner,
    /// The index is outside the menu type's declared data slots.
    InvalidIndex,
}

impl crate::NoBackend for MenuDataRefusal {
    fn no_backend() -> Self {
        Self::Unsupported
    }
}

impl MenuRef {
    /// The stack on this menu's cursor, as a place (`ItemAccess.forPlayerCursor`): nothing once
    /// the menu is gone.
    pub fn cursor(self) -> ItemPlace {
        ItemPlace::PlayerCursor { menu: self }
    }
}

/// A slot's role in the client's layout, not its server backing or acceptance rules.
/// Player indices are hotbar 0..=8, main 9..=35, armour 36..=39 (feet to head),
/// and off hand 40. Equipment belongs to the open container, never the player.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum MenuSlotKind {
    /// The player inventory index, independent of menu order.
    Player(u32),
    /// General storage, including linked storage not marked as an input.
    Storage,
    /// A crafting input grid.
    CraftingGrid,
    /// An output slot.
    Result,
    /// Equipment of the open container, such as a horse saddle.
    Equipment,
    /// A role the client layout cannot classify.
    #[default]
    Other,
    /// A machine input in the client layout, including fuel. This does not
    /// imply any particular acceptance or transfer rules.
    Input,
}

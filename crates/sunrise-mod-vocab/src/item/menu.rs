//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// One click on the local player's open menu, submitted with `MenuRef::click`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub enum MenuClick {
    /// Pick up, place or drop the carried stack; `None` means outside.
    Pickup {
        slot: Option<usize>,
        button: MouseButton,
    },
    /// Shift-click a menu-order slot into the opposite inventory.
    QuickMove { slot: usize },
    /// Exchange a menu-order slot with a hotbar slot or the offhand.
    Swap { slot: usize, with: SwapWith },
    /// Clone a menu-order slot in creative mode.
    Clone { slot: usize },
    /// Drop one item or the whole stack from a menu-order slot.
    Throw { slot: usize, whole_stack: bool },
    /// Distribute the carried stack across menu-order slots as one complete gesture.
    Drag { kind: DragKind, slots: Vec<usize> },
    /// Collect matching stacks into the carried stack.
    PickupAll { slot: usize },
}

/// The mouse button used to pick up or place a stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the two vanilla pickup buttons
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MouseButton {
    Left,
    Right,
}

/// The inventory location exchanged with a menu slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla swaps target the hotbar or the offhand
#[derive(sunrise_mod_api_macros::Wire)]
pub enum SwapWith {
    /// A hotbar index in 0..=8; other indices refuse the entire batch.
    Hotbar(u8),
    Offhand,
}

/// Whether `open_menu` or `PlayerRef::open_mod_menu` first sends the client a close for the menu
/// it replaces, as a Java `MenuProvider` chooses (`shouldTriggerClientSideContainerClosingOnOpen`).
/// Either way the old menu closes on the server, its cursor stack going back to the player's
/// inventory and `player_container_close` firing once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla's choice is a boolean
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MenuReplace {
    /// `ServerPlayer.closeContainer`, vanilla's default: the client closes the old screen first,
    /// which centres the mouse again.
    CloseFirst,
    /// `ServerPlayer.doCloseContainer`: the new screen takes the old one's place, title and rows
    /// included, and the mouse stays where it is.
    InPlace,
}

/// How a complete drag distributes the carried stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla has split, one-per-slot and creative-fill drags
#[derive(sunrise_mod_api_macros::Wire)]
pub enum DragKind {
    Split,
    One,
    /// Fill each slot in creative mode only.
    Clone,
}

/// Identity and location of a player pressing a mod container button.
// exhaustive: the game builds it with a literal; mods only read it
#[derive(sunrise_mod_api_macros::Wire)]
pub struct MenuButtonContext {
    pub player: u128,
    pub player_entity_id: i32,
    /// The block backing the menu, or None when the menu was opened without a block.
    pub pos: Option<[i32; 3]>,
    pub id: i32,
}

#[cfg(test)]
mod menu_button_context_tests {
    use super::MenuButtonContext;
    use crate::wire::{Reader, Writer};

    #[test]
    fn menu_button_context_round_trips_with_and_without_a_block_position() {
        for pos in [None, Some([12, -4, 29])] {
            let context = MenuButtonContext {
                player: 0x1234,
                player_entity_id: 57,
                pos,
                id: 8,
            };
            let mut writer = Writer::new();
            writer.put(&context);
            let decoded = Reader::decode::<MenuButtonContext>(writer.bytes()).unwrap();
            assert_eq!(decoded.player, context.player);
            assert_eq!(decoded.player_entity_id, context.player_entity_id);
            assert_eq!(decoded.pos, context.pos);
            assert_eq!(decoded.id, context.id);
        }
    }

    #[test]
    fn stacked_on_other_direction_round_trips_on_wire() {
        use super::{StackedOnOther, StackedOnOtherDirection};
        for direction in [
            StackedOnOtherDirection::Event,
            StackedOnOtherDirection::CarriedOnOther,
            StackedOnOtherDirection::OtherStackedOnMe,
        ] {
            let click = StackedOnOther::new(Some(7), Some(9), 1, 2, 3, true, None, None)
                .with_direction(direction);
            let mut writer = Writer::new();
            writer.put(&click);
            let decoded = Reader::decode::<StackedOnOther>(writer.bytes()).unwrap();
            assert_eq!(decoded, click);
            assert_eq!(decoded.direction, direction);
        }
    }
}

/// The logical side on which a menu click runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(sunrise_mod_api_macros::Wire)]
pub enum LogicalSide {
    Client,
    Server,
}
/// Which phase of a pickup click (`AbstractContainerMenu.tryItemClickBehaviourOverride`) is
/// asking `item_stacked_on_other`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// exhaustive: NeoForge's event, then vanilla's carried-on-other and other-on-me overrides
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StackedOnOtherDirection {
    /// The global `Hooks::item_stacked_on_other` event, before either item override.
    #[default]
    Event,
    /// `Item.overrideStackedOnOther`: the carried (cursor) stack's `ItemPart`, asked first.
    CarriedOnOther,
    /// `Item.overrideOtherStackedOnMe`: the clicked slot stack's `ItemPart`, asked second.
    OtherStackedOnMe,
}
/// A pickup click on a menu slot, as `item_stacked_on_other` is asked about it: the stacks are the
/// ones the click found, frozen before any listener ran. A listener answers through
/// [`StackedOnOtherResult`]; it does not write these.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StackedOnOther {
    /// The clicking player's UUID, when a player is clicking.
    pub player: Option<u128>,
    /// The clicking player's entity id, when a player is clicking.
    pub player_entity_id: Option<i32>,
    /// The clicked slot's menu index.
    pub slot: usize,
    /// Which container the slot is in, as the menu numbers them.
    pub container: usize,
    /// The slot's index in that container.
    pub container_slot: usize,
    /// A left-button click (a right-button click is not primary).
    pub primary: bool,
    /// The stack on the cursor, `None` when it is empty.
    pub carried: Option<ItemStack>,
    /// The stack in the clicked slot, `None` when it is empty.
    pub clicked: Option<ItemStack>,
    /// Which phase of `tryItemClickBehaviourOverride` is firing: the global event, the carried
    /// stack's `ItemPart` (`CarriedOnOther`), or the clicked stack's `ItemPart`
    /// (`OtherStackedOnMe`).
    pub direction: StackedOnOtherDirection,
}
impl StackedOnOther {
    /// The click `slot` (menu index) of `container`'s slot `container_slot`, with the stacks it found.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        player: Option<u128>,
        player_entity_id: Option<i32>,
        slot: usize,
        container: usize,
        container_slot: usize,
        primary: bool,
        carried: Option<ItemStack>,
        clicked: Option<ItemStack>,
    ) -> Self {
        Self {
            player,
            player_entity_id,
            slot,
            container,
            container_slot,
            primary,
            carried,
            clicked,
            direction: StackedOnOtherDirection::Event,
        }
    }

    /// This click in the given `direction` phase.
    pub fn with_direction(mut self, direction: StackedOnOtherDirection) -> Self {
        self.direction = direction;
        self
    }
}
/// Whether a listener keeps the original stack or replaces it.
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: a stack is left alone or replaced, nothing else
#[derive(sunrise_mod_api_macros::Wire)]
pub enum StackResult {
    /// Leave the original stack unchanged.
    #[default]
    Keep,
    /// Replace the stack: `None` empties it.
    Set(Option<ItemStack>),
}
/// What a listener does to the clicked slot's stack in [`StackedOnOtherResult`].
#[derive(Clone, Debug, Default, PartialEq)]
// exhaustive: a clicked slot is left alone, directly written, or taken from
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ClickedStackResult {
    /// Leave the clicked slot unchanged.
    #[default]
    Keep,
    /// Directly write the clicked slot (`Slot.set`): `None` clears the slot without running
    /// `Slot.onTake` side effects.
    Set(Option<ItemStack>),
    /// Take `count` items from the clicked slot via `Slot.safeTake` / `Slot.onTake`, running the
    /// slot's take side effects (such as furnace smelting experience, `recipes_used`, crafting
    /// remainders, or potion take hooks). Valid only in `ItemPart` overrides (refused for global
    /// listeners); `count` must be at least `1` and at most the slot's current stack size.
    /// Two accepted takes run their effects separately, in carried-then-clicked order.
    Take(u32),
}
/// A listener's answer to [`StackedOnOther`], validated once and applied once.
///
/// For global `Hooks::item_stacked_on_other` listeners (`StackedOnOtherDirection::Event`), setting
/// a stack is allowed only on a cancelled click that is also `handled` (and
/// [`ClickedStackResult::Take`] is refused); on any other answer the click is refused and nothing
/// changes.
///
/// For `ItemPart::item_stacked_on_other` overrides (`CarriedOnOther` / `OtherStackedOnMe`),
/// [`ClickedStackResult::Set`] performs a direct write (without `Slot.onTake` side effects, even
/// when clearing a result/output slot to `None`), while [`ClickedStackResult::Take`] takes from
/// the clicked slot through `Slot.safeTake` / `Slot.onTake`. A replaced stack must be a valid
/// stack (a known item, a count of at least one within the item's maximum).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct StackedOnOtherResult {
    /// `ItemStackedOnOtherEvent.setCancellationResult`: on a cancelled click, whether the click
    /// is done (`true`: the listener's work is all of it) or the plain pickup still runs.
    pub handled: bool,
    /// The cursor after the click.
    pub carried: StackResult,
    /// The clicked slot edit after the click.
    pub clicked: ClickedStackResult,
}
/// The inputs and vanilla result of `AnvilMenu.createResult`, borrowed for
/// the `anvil_update` callback. Edits by earlier listeners do not change these values.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the converted stacks and rename text for the hook call.
pub struct AnvilUpdateInputs<'a> {
    /// The stack in the left input slot. The hook is not asked without one.
    pub left: &'a ItemStack,
    /// The stack in the right input slot, if present.
    pub right: Option<&'a ItemStack>,
    /// The rename box's text, `None` if the client sent none.
    pub name: Option<&'a str>,
    /// The result calculated by vanilla, if any.
    pub vanilla_output: Option<&'a ItemStack>,
    /// The level cost calculated by vanilla.
    pub vanilla_xp_cost: i32,
    /// The number of material items vanilla would charge from the right slot.
    pub vanilla_material_cost: i32,
}

/// The shared answer to `anvil_update`. It starts at `Keep` and vanilla's costs.
/// The game applies the final edits after dispatch; cancellation leaves no output
/// and zero costs, while an invalid replacement output is logged and ignored
/// (keeping vanilla's output).
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal
// no wire: staged edits are applied in-process at the end of the hook call.
pub struct AnvilUpdateEdits {
    /// Keep vanilla's output, or set a replacement. `Set(None)` clears it.
    /// A replacement differing from vanilla must be a valid stack; an invalid
    /// stack is logged and ignored, leaving vanilla's output intact.
    pub output: StackResult,
    /// Signed level cost during listener dispatch (`AnvilUpdateEvent.setXpCost`).
    /// Negative values clamp to `0` when applied (`AnvilMenu.setCost`), and a
    /// zero cost prevents taking the result (`AnvilMenu.mayPickup`).
    pub xp_cost: i32,
    /// Units consumed from the right slot. Negative values clamp to zero;
    /// zero or a value exceeding the stack consumes all of it on take.
    pub material_cost: i32,
}
/// The inputs and vanilla result of `GrindstoneMenu.createResult`, borrowed for
/// the `grindstone_place` callback. Edits by earlier listeners do not change these values.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the converted stacks for the hook call.
pub struct GrindstonePlaceInputs<'a> {
    /// The stack in the top input slot, if present.
    pub top: Option<&'a ItemStack>,
    /// The stack in the bottom input slot, if present.
    pub bottom: Option<&'a ItemStack>,
    /// The result calculated by vanilla, if any.
    pub vanilla_output: Option<&'a ItemStack>,
}

/// The shared answer to `grindstone_place`, initially no override and `-1` XP.
/// Cancellation is checked before converting `output`; an invalid replacement is logged
/// and ignored so vanilla's result stays intact.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal
// no wire: staged edits are applied in-process at the end of the hook call.
pub struct GrindstonePlaceEdits {
    /// Leave vanilla's result and experience with `None`, or replace the result with a
    /// valid stack. To clear the result, cancel the hook instead.
    pub output: Option<ItemStack>,
    /// Experience to award when the replacement is taken. Any negative value falls
    /// through to vanilla's award; nonnegative values set the award.
    pub xp: i32,
}
/// The input stacks before the grindstone result was taken, borrowed for the
/// `grindstone_take` callback. Edits by earlier listeners do not change this snapshot.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the converted pre-take stacks for the hook call.
pub struct GrindstoneTakeInputs<'a> {
    /// The stack in the top input slot before the take, if present.
    pub top: Option<&'a ItemStack>,
    /// The stack in the bottom input slot before the take, if present.
    pub bottom: Option<&'a ItemStack>,
}

/// The shared answer to `grindstone_take`. Both remainders start at `None`;
/// cancellation is checked before converting remainders, and an invalid remainder is
/// logged and ignored as an empty slot.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal
// no wire: staged edits are applied in-process at the end of the hook call.
pub struct GrindstoneTakeEdits {
    /// Experience to award, initially the resolved award (`>= 0`, from `grindstone_place`
    /// or vanilla's pre-rolled award). Non-positive values (`<= 0`) spawn no experience orbs.
    pub xp: i32,
    /// A valid replacement for the top input, or `None` to empty the slot.
    pub new_top: Option<ItemStack>,
    /// A valid replacement for the bottom input, or `None` to empty the slot.
    pub new_bottom: Option<ItemStack>,
}
/// The anvil inputs and result before the take. Both `anvil_craft_pre` and
/// `anvil_craft_post` borrow this same snapshot for their callback.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
// no wire: borrows the pre-take snapshot for this hook call.
pub struct AnvilCraftInputs<'a> {
    /// The stack in the left slot before the take, if present.
    pub left: Option<&'a ItemStack>,
    /// The stack in the right slot before the take, if present.
    pub right: Option<&'a ItemStack>,
    /// The result stack that was taken.
    pub output: &'a ItemStack,
}

/// One of a menu's containers: the key a `menu_slot_rules` ask names its slot's container by
/// (`event.container`, with `event.slot` indexing inside it) and [`MenuSlotContext::slot`] reads
/// by. A plain menu has `Main` and `Player`; a mod menu type with a crafting grid keeps its grid
/// slots in `Grid` and its crafted result in `Result` (slot 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum MenuContainer {
    /// The menu's own slots (a chest's, a machine's).
    #[default]
    Main,
    /// A crafting grid's slots, row by row.
    Grid,
    /// A crafting grid's result slot.
    Result,
    /// The player's inventory, using vanilla `Inventory` indices: `0..36` inventory, `36..40`
    /// armor (feet, legs, chest, head) and `40` offhand.
    Player,
}
/// The menu's player, its block entity and a read-only view of its live slots for one
/// `menu_slot_rules` ask, on either the server or the predicting client. Read only the slots
/// needed to answer the rule; the view cannot outlive the ask.
// no wire: this view borrows the menu's live slots during a click.
#[non_exhaustive]
pub struct MenuSlotContext<'a> {
    /// The UUID of the player whose menu it is; None where no player is known.
    pub player: Option<u128>,
    /// The block entity the menu belongs to; None when the menu is not a block's
    /// (opened by a mod with no block, a vehicle's, the player's own inventory).
    pub pos: Option<[i32; 3]>,
    read: &'a dyn Fn(MenuContainer, usize) -> Option<ItemStack>,
}
impl<'a> MenuSlotContext<'a> {
    /// Lend a read of the menu's live slots, with the player and block position of this ask.
    /// The reader returns `None` for empty, absent or unrepresentable slots.
    pub fn new(
        player: Option<u128>,
        pos: Option<[i32; 3]>,
        read: &'a dyn Fn(MenuContainer, usize) -> Option<ItemStack>,
    ) -> Self {
        Self { player, pos, read }
    }

    /// Read one slot at its backing index, converting just that stack to an API value.
    /// Returns `None` if it is empty, absent or cannot be represented. For `Player`, indices follow
    /// vanilla `Inventory`: `0..36` inventory, `36..40` armor (feet-to-head) and `40` offhand.
    pub fn slot(&self, container: MenuContainer, index: usize) -> Option<ItemStack> {
        (self.read)(container, index)
    }
}

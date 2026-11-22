//! Whole stacks for server mods: one given to a player as `/give` gives it
//! (`player_give_stack`), and one read from SNBT as a saved item is (`item_from_snbt`).
use super::Game;
use mod_api::{PlayerDataRefusal, PlayerRef};
use sunrise_client::{items::ItemStack, lan::playerdata};

/// The most SNBT text `item_from_snbt` reads: `NbtAccounter`'s default quota, 2 MiB.
const MAX_SNBT_BYTES: usize = 2 * 1024 * 1024;

impl Game<'_> {
    /// `GiveCommand.giveItem` for one stack: `Inventory.add`, the rest thrown for the player
    /// alone to pick up in their level, even when the hook is running in another one.
    pub(super) fn player_give_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
    ) -> Result<Option<ItemStack>, PlayerDataRefusal> {
        self.player_write_gate(player)?;
        let owner = Some(playerdata::uuid_ints(player.uuid()));
        let mut rest = None;
        let ran = self.native_as_player_handle(player, |game| {
            rest = game.give_acting_stack(stack, owner);
            // containerMenu.broadcastChanges: InventoryChangeTrigger.
            if game.player.is_some() {
                game.after_commit(super::transactions::After::InventoryTrigger(player.uuid()));
            }
        });
        if ran {
            Ok(rest)
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }

    /// `ItemStack.CODEC` reading the compound `snbt` stands for (`TagParser`): only that
    /// compound and whitespace. At most 2 MiB and 512 levels: vanilla's SNBT parser has
    /// neither limit (deep text ends in a stack overflow); they are this API's bound so
    /// mod-supplied text cannot overflow the parser.
    pub(super) fn item_from_snbt(&self, snbt: &str) -> Option<ItemStack> {
        if snbt.len() > MAX_SNBT_BYTES {
            return None;
        }
        let (tag, end) = sunrise_core::nbt::parse_snbt(snbt).ok()?;
        if !snbt.as_bytes()[end..].iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        let stack = sunrise_items::item_nbt::item_from_persistent_nbt(&tag)?;
        self.items().get(&stack.item).is_some().then_some(stack)
    }
}

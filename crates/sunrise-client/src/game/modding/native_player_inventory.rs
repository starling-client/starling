//! A player's inventory by vanilla's slot numbers (`Inventory.getItem`/`setItem`).
//! Reads lend the current slots; writes go through the authoritative server.
use super::{
    Game,
    api_values::{project_stack, stack_read_metadata},
    commands::items::Slot,
    native_server::PlayerRead,
};
use mod_api::{InventoryView, PlayerDataRefusal, PlayerRef};
use sunrise_client::items::ItemStack;
use sunrise_items::equipment::{BODY, SADDLE};

/// `Inventory`'s slot numbers past the 36 main slots (`EQUIPMENT_SLOT_MAPPING`).
const SLOTS: u32 = 43;

/// 0..35 main (0..8 hotbar), 36..39 feet to head, 40 offhand, 41 body, 42 saddle.
/// ItemPlace uses 100..103 for armour and -106 for offhand; InventoryMenu has no 41/42.
/// The slot the game keeps `Inventory` slot `slot` in.
fn inventory_slot(slot: u32) -> Option<Slot> {
    Some(match slot {
        0..36 => Slot::Inv(slot as usize),
        // `armor` is head first: 36 feet is its last.
        36..40 => Slot::Armor((39 - slot) as usize),
        40 => Slot::Offhand,
        41 => Slot::Equipment(BODY),
        42 => Slot::Equipment(SADDLE),
        _ => return None,
    })
}

/// The authority refusals shared by writes to a server player's own state.
pub(super) enum PlayerWriteRefusal {
    Stale,
    Client,
    Dead,
}

impl From<PlayerWriteRefusal> for PlayerDataRefusal {
    fn from(error: PlayerWriteRefusal) -> Self {
        match error {
            PlayerWriteRefusal::Stale => Self::Stale,
            PlayerWriteRefusal::Client => Self::Client,
            PlayerWriteRefusal::Dead => Self::Dead,
        }
    }
}

/// Lend `Inventory.getItem` over a player's 43 vanilla inventory slots.
pub(super) fn lend_inventory_view(
    inventory: &[Option<ItemStack>],
    armor: &[Option<ItemStack>; 4],
    offhand: &Option<ItemStack>,
    equipment: &sunrise_items::equipment::Equipment,
    read: &mut dyn for<'a> FnMut(InventoryView<'a>),
) {
    let stack_at = |slot| match inventory_slot(slot)? {
        Slot::Inv(i) => inventory.get(i)?.as_ref(),
        Slot::Armor(i) => armor[i].as_ref(),
        Slot::Offhand => offhand.as_ref(),
        Slot::Equipment(i) => equipment.get(i),
        _ => None,
    };
    read(InventoryView::new(
        0..SLOTS,
        &|slot| stack_read_metadata(stack_at(slot)?).ok(),
        &|slot, metadata| project_stack(stack_at(slot)?, metadata).ok(),
    ));
}

impl Game<'_> {
    /// Lend `Inventory.getItem` over the resident slots.
    pub(super) fn player_inventory_slots(
        &self,
        player: PlayerRef,
        read: &mut dyn for<'a> FnMut(InventoryView<'a>),
    ) -> Option<()> {
        let (inventory, armor, offhand, equipment) = match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => {
                (&self.inventory, &self.armor, &self.offhand, &self.equipment)
            }
            PlayerRead::Guest(g) => (&g.inventory, &g.armor, &g.offhand, &g.equipment),
        };
        lend_inventory_view(inventory, armor, offhand, equipment, read);
        Some(())
    }

    /// Why a `give_stack` action is refused before it starts: a client, a handle
    /// with no online player, or a dead player.
    pub(super) fn player_write_gate(&self, player: PlayerRef) -> Result<(), PlayerWriteRefusal> {
        if self.remote.is_some() {
            return Err(PlayerWriteRefusal::Client);
        }
        match self.player_handle_read(player) {
            None => Err(PlayerWriteRefusal::Stale),
            Some(PlayerRead::Guest(g)) if g.dead => Err(PlayerWriteRefusal::Dead),
            Some(_) => Ok(()),
        }
    }

    /// `Inventory.setItem`, in the player's turn: the tick's inventory sync tells the client.
    pub(super) fn player_set_inventory_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal> {
        let turn = self.native_resident_player_turn(player)?;
        let target = inventory_slot(slot).ok_or(PlayerDataRefusal::SlotOutOfRange {
            slot,
            last: SLOTS - 1,
        })?;
        let set = turn.run(self, |game| {
            let set = game.slot_set(&target, stack.filter(|s| s.count > 0));
            if set && game.player.is_some() {
                game.after_commit(super::transactions::After::InventoryTrigger(player.uuid()));
            }
            set
        });
        if set == Some(true) {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }
}

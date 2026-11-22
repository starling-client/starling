//! Live living-entity weapon reads for death loot.
use super::{Game, commands::Target, native_living_refs::lend_equipment};
use mod_api::{EquipmentSlot, ItemStackView, LivingRef};
use sunrise_client::mobs::finalize::EquipmentRead;

impl Game<'_> {
    pub(super) fn living_weapon_item(
        &self,
        living: LivingRef,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        let target = self.native_living_target(living)?;
        let motion = match target {
            Target::Player => self.player.as_ref(),
            Target::Guest(conn) => self.command_guest(conn)?.motion.as_deref(),
            _ => None,
        };
        if let Some(player) = motion.filter(|player| player.auto_spin_attack_ticks > 0)
            && let Some(stack) = &player.auto_spin_attack_item
        {
            return lend_equipment(EquipmentRead::resident(stack), read);
        }
        lend_equipment(
            self.equipment_stack_for(target, EquipmentSlot::MainHand)?,
            read,
        )
    }
}

//! `Mob.aiStep`'s looting loop: a mob with `canPickUpLoot` takes what lies
//! within its box grown by `getPickupReach` (1, 0, 1) and wears or holds it
//! when it beats what it has (`equipItemIfPossible`), dropping the replaced
//! stack by its drop chance. The species whose `pickUpItem` does something
//! else keep their own loops (`pandas.rs`, `piglins.rs`, `villagers.rs`,
//! `allays.rs`, `sulfur_cubes.rs`).
use super::Game;
#[cfg(test)]
use sunrise_client::mobs::finalize;
use sunrise_client::{
    entities::ItemEntity,
    items::ItemStack,
    mobs::{
        Mob,
        equip::{self, Facts, Slot},
    },
};
use sunrise_gameplay::mob_looting::LootingLevel;

impl LootingLevel for Game<'_> {
    fn grief_hooked(&self) -> bool {
        Game::grief_hooked(self)
    }
    fn equippable_in_slot(&self, stack: &ItemStack, slot: &str, kind: &str) -> bool {
        equip::equippable_in_slot(self.items(), stack, slot, kind)
    }
    fn can_equip(
        &self,
        stack: &ItemStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        let prepared = Game::equip_stack(self, stack);
        Game::can_equip(self, &prepared, slot, entity, kind, data)
    }
    fn mob_equip_event(&mut self, id: u32, at: [f64; 3], equip: bool) {
        Game::mob_equip_event(self, id, at, equip);
    }
    fn stack_facts(&self, index: usize, stack: &ItemStack, slot: Slot) -> Facts {
        Game::stack_facts(self, &self.mobs[index], stack, slot)
    }
}

impl Game<'_> {
    /// `Mob.aiStep`'s looting block, once per mob per tick.
    pub(super) fn tick_mob_looting(&mut self) {
        sunrise_gameplay::mob_looting::tick_mob_looting(self);
    }

    /// `Mob.equipItemIfPossible`, as the tests ask it.
    #[cfg(test)]
    fn equip_item_if_possible(&mut self, index: usize, stack: &ItemStack) -> Option<u32> {
        sunrise_gameplay::mob_looting::equip_item_if_possible(self, index, stack)
    }

    /// The numbers `Mob.compareArmor` and `compareWeapons` read off `stack`
    /// in `slot` for the mob `m`.
    pub(super) fn stack_facts(&self, m: &Mob, stack: &ItemStack, slot: Slot) -> Facts {
        let (armor, toughness, attack) = self.attributes(m, stack, slot);
        Facts {
            armor,
            armor_toughness: toughness,
            attack_damage: attack,
            enchantments: stack.enchantments().len(),
            damage: stack.damage as i32,
            custom_name: stack.custom_name().is_some(),
            prevents_armor_change: world_stations!(self)
                .enchantments
                .prevents_armor_change(stack),
        }
    }

    /// `Mob.getApproximateAttributeWith` for `stack` in `slot`: each
    /// attribute starts from the mob's own value. (armour, toughness, damage)
    fn attributes(&self, m: &Mob, stack: &ItemStack, slot: Slot) -> (f64, f64, f64) {
        let modifiers = sunrise_client::item_attributes::attribute_modifiers(stack, self.items());
        let modifiers = &*modifiers;
        (
            equip::attribute_with(modifiers, "minecraft:armor", f64::from(m.kind.armor), slot),
            equip::attribute_with(
                modifiers,
                "minecraft:armor_toughness",
                f64::from(m.kind.armor_toughness),
                slot,
            ),
            equip::attribute_with(
                modifiers,
                "minecraft:attack_damage",
                f64::from(m.attrs.attack_damage),
                slot,
            ),
        )
    }

    /// `LivingEntity.onItemPickup`: the advancement of whoever threw the item,
    /// whichever mob took it.
    pub(super) fn mob_item_pickup_trigger(&mut self, index: usize, e: &ItemEntity) {
        let Some(owner) = e.thrower else {
            return;
        };
        let stack = e.stack.clone();
        let target = self.mob_entity(&self.mobs[index]);
        self.for_each_player(|g| {
            if g.acting_uuid() == owner {
                g.trigger_item_entity("minecraft:thrown_item_picked_up_by_entity", &stack, &target);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// What the part of an iron helmet answers to `can_equip`: yes for one entity type only,
    /// and only while that entity stands at or above y 64, read through the `Host`.
    #[derive(Clone)]
    struct OnlyFor(&'static str, Arc<Mutex<Vec<(String, u32)>>>);
    #[mod_api::sunrise_part]
    impl mod_api::ItemPart for OnlyFor {
        fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
            let entity = event.entity.entity();
            (self.1.lock().unwrap()).push((event.slot.to_owned(), entity.id()));
            let standing = entity.position(host).is_some_and(|[_, y, _]| y >= 64.0);
            event.can_equip = event.kind == self.0 && standing;
        }
    }
    struct Hatter(OnlyFor);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Hatter {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let helmet = r.item_key("minecraft:iron_helmet");
            r.item_part(helmet, self.0.clone());
        }
    }

    /// `Mob.equipItemIfPossible` asks `isEquippableInSlot` first, and `IItemExtension.canEquip`
    /// is that answer: a part of the iron helmet that accepts only skeletons keeps it off a
    /// zombie that would take it, and puts it on a skeleton, unless the skeleton stands lower
    /// than the part allows. Items without a part follow their component.
    #[test]
    #[ignore = "daily"]
    fn a_can_equip_part_decides_which_mob_wears_a_picked_up_item() {
        use sunrise_core::geometry::Vec3;

        let asked = Arc::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "hatter",
            Box::new(Hatter(OnlyFor("minecraft:skeleton", Arc::clone(&asked)))),
        )]
        .into();
        crate::entry::install_client_hooks();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let mobs = [
                    (1, "minecraft:zombie", 64.0),
                    (2, "minecraft:skeleton", 64.0),
                    (3, "minecraft:skeleton", 60.0),
                ];
                for (id, name, y) in mobs {
                    let kind = game.res.mob_types().get(name).unwrap().clone();
                    game.mobs
                        .push(Mob::new(id, kind, Vec3::new(0.5, y, 0.5), 0.0));
                }
                let worn = |game: &Game<'_>, index: usize, slot: Slot| {
                    finalize::item(&game.mobs[index], slot.index()).map(|item| item.to_string())
                };
                let helmet = ItemStack::new("minecraft:iron_helmet", 1);
                crate::game::equip_answers::take_equip_conversions();
                // The zombie is refused, and nothing it was asked about changed.
                assert_eq!(game.equip_item_if_possible(0, &helmet), None);
                assert_eq!(worn(game, 0, Slot::Head), None);
                assert_eq!(worn(game, 0, Slot::MainHand), None);
                assert_eq!(
                    std::mem::take(&mut *asked.lock().unwrap()),
                    [("head".to_owned(), 1)]
                );
                // The skeleton is accepted.
                assert_eq!(game.equip_item_if_possible(1, &helmet), Some(1));
                assert_eq!(
                    worn(game, 1, Slot::Head).as_deref(),
                    Some("minecraft:iron_helmet")
                );
                assert_eq!(
                    std::mem::take(&mut *asked.lock().unwrap()),
                    [("head".to_owned(), 2)]
                );
                // A skeleton standing below y 64 is refused: the part reads where it is.
                assert_eq!(game.equip_item_if_possible(2, &helmet), None);
                assert_eq!(worn(game, 2, Slot::Head), None);
                assert_eq!(
                    std::mem::take(&mut *asked.lock().unwrap()),
                    [("head".to_owned(), 3)]
                );
                assert_eq!(
                    crate::game::equip_answers::take_equip_conversions(),
                    3,
                    "one preparation per mob pickup attempt"
                );
                // No part for a diamond helmet: the zombie takes it as vanilla.
                let diamond = ItemStack::new("minecraft:diamond_helmet", 1);
                assert_eq!(game.equip_item_if_possible(0, &diamond), Some(1));
                assert_eq!(
                    worn(game, 0, Slot::Head).as_deref(),
                    Some("minecraft:diamond_helmet")
                );
                assert!(asked.lock().unwrap().is_empty());
                assert_eq!(crate::game::equip_answers::take_equip_conversions(), 0);
            });
        });
    }

    /// `getEquipmentSlotForItem` and `isEquippableInSlot` read the stack's own `equippable`, with
    /// no part installed: a zombie holds an iron helmet whose component is removed in its main
    /// hand, wears one overridden for the feet on its feet, and takes none whose
    /// `allowed_entities` leaves zombies out, an explicitly empty list included.
    #[test]
    #[ignore = "daily"]
    fn a_mob_reads_the_picked_up_stacks_own_equippable() {
        use sunrise_core::{geometry::Vec3, nbt::Tag};

        let helmet = |fields: Option<Vec<(&str, Tag)>>| {
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
            let (id, value) = match fields {
                Some(fields) => ("minecraft:equippable", Tag::compound(fields)),
                None => ("!minecraft:equippable", Tag::Compound(Default::default())),
            };
            helmet.data_mut().set_other(id, Some(value));
            helmet
        };
        let slot = |name: &str| ("slot", Tag::String(name.into()));
        let allowed = |list: Vec<Tag>| ("allowed_entities", Tag::List(list));
        crate::game::native_entity_refs::tests::with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            for id in 1..=4 {
                let at = Vec3::new(0.5 + f64::from(id) * 4.0, 64.0, 0.5);
                game.mobs.push(Mob::new(id, kind.clone(), at, 0.0));
            }
            let worn = |game: &Game<'_>, index: usize| {
                [Slot::MainHand, Slot::Head, Slot::Feet]
                    .into_iter()
                    .find(|slot| finalize::item(&game.mobs[index], slot.index()).is_some())
            };
            assert_eq!(game.equip_item_if_possible(0, &helmet(None)), Some(1));
            assert_eq!(worn(game, 0), Some(Slot::MainHand));
            let feet = helmet(Some(vec![slot("feet")]));
            assert_eq!(game.equip_item_if_possible(1, &feet), Some(1));
            assert_eq!(worn(game, 1), Some(Slot::Feet));
            let skeletons = vec![Tag::String("skeleton".into())];
            let skeletons = helmet(Some(vec![slot("head"), allowed(skeletons)]));
            assert_eq!(game.equip_item_if_possible(2, &skeletons), None);
            assert_eq!(worn(game, 2), None);
            let nobody = helmet(Some(vec![slot("head"), allowed(Vec::new())]));
            assert_eq!(game.equip_item_if_possible(3, &nobody), None);
            assert_eq!(worn(game, 3), None);
        });
    }
}

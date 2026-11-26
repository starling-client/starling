//! `IItemExtension.canEquip` on the game's side (`native_can_equip`). The dispenser, a mob's
//! pickup and the two inventory menus ask it, each with the game as the read-only `Host` and the
//! target as a handle. A stack can be converted once for the parts and lent to every slot and
//! entity asked about it in a synchronous check.
use super::*;
use crate::game::api_values::TryIntoApi;
use sunrise_client::mobs::equip::CanEquip;

/// A stack converted once for the parts of its item, ready for any number of `can_equip`
/// questions in a synchronous check. `DataOnly` when no part of the item listens (the component's
/// answer stands and nothing is converted); `Refused` when a listening stack cannot be converted.
pub(in crate::game) enum EquipStack {
    DataOnly,
    Refused,
    Ready(Box<mod_api::ItemStack>),
}

impl Game<'_> {
    pub(in crate::game) fn equip_stack(&self, stack: &ItemStack) -> EquipStack {
        if !mod_api::__private::item_listens(mod_api::HookId::CanEquip, &stack.item) {
            return EquipStack::DataOnly;
        }
        #[cfg(test)]
        EQUIP_CONVERSIONS.with(|count| count.set(count.get() + 1));
        match stack.try_into_api() {
            Ok(stack) => EquipStack::Ready(Box::new(stack)),
            Err(_) => EquipStack::Refused,
        }
    }

    /// `LivingEntity.isEquippableInSlot` with `IItemExtension.canEquip`: `data` is the stack's
    /// `equippable` answer for the entity `entity` of type `kind` in `slot`, and the parts of
    /// `stack`'s item may change it. When a part would be asked and `entity` is no live entity of
    /// this side (a missing or stale handle), the slot is refused.
    pub(in crate::game) fn can_equip(
        &self,
        stack: &EquipStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        let stack = match stack {
            EquipStack::DataOnly => return data,
            EquipStack::Refused => return false,
            EquipStack::Ready(stack) => stack,
        };
        let Some(entity) = self.entity(entity) else {
            return false;
        };
        let native = crate::game::native_hooks::NativeHookHost { host: self };
        let mut can_equip = data;
        native.native_can_equip(
            stack,
            slot,
            mod_api::LivingRef::new(entity),
            kind,
            &mut can_equip,
        );
        can_equip
    }

    /// `can_equip` lent to a menu click that runs while the game is at hand (the player's own
    /// inventory menu, predicted or authoritative).
    pub(in crate::game) fn menu_can_equip(&self) -> MenuEquip<'_, '_> {
        MenuEquip { game: self }
    }
}

/// The game lent to a menu click as its `CanEquip`.
pub(in crate::game) struct MenuEquip<'g, 'a> {
    game: &'g Game<'a>,
}

/// Call-scoped `CanEquip` that lazily converts one unchanged `ItemStack` at most once across a
/// synchronous candidate loop. Rust's `&'s ItemStack` borrow prevents mutation during the loop.
struct ScopedMenuEquip<'g, 'a, 's> {
    game: &'g Game<'a>,
    stack: &'s ItemStack,
    prepared: std::cell::OnceCell<EquipStack>,
}

impl CanEquip for ScopedMenuEquip<'_, '_, '_> {
    fn listens(&self, item: &str) -> bool {
        mod_api::__private::item_listens(mod_api::HookId::CanEquip, item)
    }
    fn can_equip(
        &self,
        stack: &ItemStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        if std::ptr::eq(stack, self.stack) {
            let prepared = self
                .prepared
                .get_or_init(|| self.game.equip_stack(self.stack));
            self.game.can_equip(prepared, slot, entity, kind, data)
        } else {
            let prepared = self.game.equip_stack(stack);
            self.game.can_equip(&prepared, slot, entity, kind, data)
        }
    }
    fn with_stack(&self, _stack: &ItemStack, body: &mut dyn FnMut(&dyn CanEquip)) {
        body(self);
    }
}

impl CanEquip for MenuEquip<'_, '_> {
    fn listens(&self, item: &str) -> bool {
        mod_api::__private::item_listens(mod_api::HookId::CanEquip, item)
    }
    fn can_equip(
        &self,
        stack: &ItemStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        let prepared = self.game.equip_stack(stack);
        self.game.can_equip(&prepared, slot, entity, kind, data)
    }
    fn with_stack(&self, stack: &ItemStack, body: &mut dyn FnMut(&dyn CanEquip)) {
        let scoped = ScopedMenuEquip {
            game: self.game,
            stack,
            prepared: std::cell::OnceCell::new(),
        };
        body(&scoped);
    }
}

#[cfg(test)]
thread_local! { static EQUIP_CONVERSIONS:std::cell::Cell<usize>=const{std::cell::Cell::new(0)}; }
#[cfg(test)]
pub(in crate::game) fn take_equip_conversions() -> usize {
    EQUIP_CONVERSIONS.with(|count| count.replace(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The part of an iron helmet: it lets the helmet on the feet only, and counts the asks.
    #[derive(Clone, Default)]
    struct FeetOnly(std::sync::Arc<AtomicUsize>);
    #[mod_api::sunrise_part]
    impl mod_api::ItemPart for FeetOnly {
        fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
            let entity = event.entity.entity();
            assert!(entity.position(host).is_some());
            assert!(entity.kind(host).is_some());
            let _health = event.entity.health(host);
            let _alive = event.entity.alive(host);
            let _equipment =
                event
                    .entity
                    .with_equipment(host, mod_api::EquipmentSlot::Head, |_| ());
            let _hand = event
                .entity
                .with_hand_item(host, mod_api::Hand::Main, |_| ());
            self.0.fetch_add(1, Ordering::Relaxed);
            event.can_equip = event.slot == "feet";
        }
    }
    /// A part of stone that allows every armor slot and counts asks, for multi-slot QuickCraft
    /// tests.
    #[derive(Clone, Default)]
    struct AllArmor(std::sync::Arc<AtomicUsize>);
    #[mod_api::sunrise_part]
    impl mod_api::ItemPart for AllArmor {
        fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
            assert!(event.entity.entity().position(host).is_some());
            self.0.fetch_add(1, Ordering::Relaxed);
            event.can_equip = true;
        }
    }
    struct Cobbler(FeetOnly, AllArmor);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Cobbler {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let helmet = r.item_key("minecraft:iron_helmet");
            r.item_part(helmet, self.0.clone());
            let stone = r.item_key("minecraft:stone");
            r.item_part(stone, self.1.clone());
        }
    }

    /// A component-rich stack of `item` with count `count` whose conversion for the parts
    /// allocates.
    fn rich_item(item: &str, count: u32) -> ItemStack {
        use sunrise_core::nbt::Tag;
        let mut stack = ItemStack::new(item, count);
        stack.set_crafting_enchantments(vec![("minecraft:protection".into(), 4)]);
        let data = stack.data_mut();
        data.custom_name = Some(Tag::String("Crown".into()));
        let component = Tag::compound([("slot", Tag::String("head".into()))]);
        data.set_other("minecraft:equippable", Some(component));
        stack
    }

    /// An iron helmet with enchantments, a name and its own `equippable`: a stack whose
    /// conversion for the parts allocates.
    fn rich_helmet() -> ItemStack {
        rich_item("minecraft:iron_helmet", 1)
    }

    /// Runs `test` on a game with the cobbler's part installed, given what it counted.
    fn with_cobbler(test: impl FnOnce(&mut Game<'_>, &AtomicUsize)) {
        installed(|asked| {
            crate::game::native_entity_refs::tests::with_game(|game| {
                // The player in place, admitted as a joined one is, so its handle resolves.
                let at = sunrise_core::geometry::Vec3::new(0.5, 64.0, 0.5);
                let mut body = sunrise_client::motion::PlayerMotion::new(game.motion, at).unwrap();
                body.entity_id = 7;
                game.player = Some(body);
                game.bind_local_player_residency();
                test(game, asked)
            });
        });
    }

    /// Runs `test` with the cobbler's mod installed, given what its part counted.
    fn installed(test: impl FnOnce(&AtomicUsize)) {
        let part = FeetOnly::default();
        let all_armor = AllArmor(std::sync::Arc::clone(&part.0));
        let cobbler = mod_api::ModEntry::new("cobbler", Box::new(Cobbler(part.clone(), all_armor)));
        let mods: mod_api::Mods = vec![cobbler].into();
        crate::entry::install_client_hooks();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || test(&part.0));
    }

    /// Asked once per candidate slot through a local `MenuEquip::with_stack` scope, a warmed
    /// `can_equip` allocates nothing and converts the stack only once across all candidates,
    /// reading live entity state through the read-only host.
    #[test]
    #[ignore = "daily"]
    fn warmed_can_equip_questions_allocate_nothing() {
        with_cobbler(|game, asked| {
            let (helmet, other) = (rich_helmet(), ItemStack::new("minecraft:diamond_helmet", 1));
            game.armor[0] = Some(rich_helmet());
            let (player, kind) = (game.own_entity_id() as u32, "minecraft:player");
            let slots = ["head", "chest", "legs", "feet"];
            let feet_only = [false, false, false, true];
            let lent = game.menu_can_equip();
            let (answers, allocations) = sunrise_threads::allocation_count::measure(|| {
                slots.map(|slot| lent.can_equip(&other, slot, player, kind, slot == "head"))
            });
            assert_eq!((answers, allocations), ([true, false, false, false], 0));
            take_equip_conversions();
            lent.with_stack(&helmet, &mut |scoped| {
                assert!(!scoped.can_equip(&helmet, "head", player, kind, true));
                assert_eq!(take_equip_conversions(), 1);
                let (answers, allocations) = sunrise_threads::allocation_count::measure(|| {
                    slots.map(|slot| scoped.can_equip(&helmet, slot, player, kind, slot == "head"))
                });
                assert_eq!((answers, allocations), (feet_only, 0));
                assert_eq!(take_equip_conversions(), 0);
            });
            assert_eq!(asked.load(Ordering::Relaxed), 1 + 4);
        });
    }

    /// A guest's right click with a bundle on the cursor on an empty armour slot takes the
    /// bundle's next stack out into it (`BundleItem.overrideStackedOnOther`), and the host's
    /// authoritative menu asks the parts about that inner stack: the cobbler's part keeps the iron
    /// helmet inside off the head, and the bundle, the armour and the inventory stay as they were;
    /// it lets it onto the feet, and the bundle is empty after.
    #[test]
    #[ignore = "daily"]
    fn a_guests_bundle_puts_on_what_the_part_allows() {
        use sunrise_client::bundle::Contents;
        use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
        installed(|asked| {
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.reset_entity_residency();
                let mut bundle = ItemStack::new("minecraft:bundle", 1);
                let mut contents = Contents::of(game.items(), &bundle).unwrap();
                let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
                assert_eq!(contents.try_insert(game.items(), &mut helmet, true), 1);
                contents.store(game.items(), &mut bundle);
                let held = |game: &Game<'_>| {
                    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                    let name = |s: &Option<ItemStack>| s.as_ref().map(|s| s.item.to_string());
                    let inside = (guest.carried.as_ref())
                        .and_then(|b| Contents::of(game.items(), b))
                        .map(|c| {
                            c.items()
                                .iter()
                                .map(|s| s.item.to_string())
                                .collect::<Vec<_>>()
                        });
                    let inventory = guest.inventory.iter().filter(|s| s.is_some()).count();
                    (guest.armor.each_ref().map(name), inside, inventory)
                };
                // Right clicks (button 1) of the pickup mode on menu slots 5 (head) and 8 (feet).
                let click = |game: &mut Game<'_>, slot: i16| {
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.carried = Some(bundle.clone());
                    });
                    let click = ContainerClick {
                        container: 0,
                        state: 0,
                        slot,
                        button: 1,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    };
                    game.lan_guest_packet(12, Serverbound::ContainerClick(click));
                };
                let helmet = "minecraft:iron_helmet".to_owned();
                take_equip_conversions();
                click(game, 5);
                assert_eq!(take_equip_conversions(), 1);
                assert_eq!(
                    held(game),
                    ([None, None, None, None], Some(vec![helmet.clone()]), 0)
                );
                click(game, 8);
                assert_eq!(take_equip_conversions(), 1);
                assert_eq!(
                    held(game),
                    ([None, None, None, Some(helmet)], Some(Vec::new()), 0)
                );
                // The part was asked about the helmet taken out of the bundle.
                assert!(asked.load(Ordering::Relaxed) > 0);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn local_shift_click_prepares_once_and_preserves_a_refused_helmet() {
        with_cobbler(|game, asked| {
            use sunrise_client::menu::{Input, Kind, Menu};
            let lent = game.menu_can_equip();
            let mut env = game.menu_env();
            env.can_equip = Some(&lent);
            let mut menu = Menu::new(Kind::Inventory);
            let helmet = rich_helmet();
            menu.set_slot(&env, 9, Some(helmet.clone()));
            take_equip_conversions();
            menu.click(&env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(take_equip_conversions(), 1);
            assert_eq!(asked.load(Ordering::Relaxed), 1);
            assert_eq!(menu.slot(9), Some(&helmet));
            assert!(menu.slot(5).is_none());

            // Multi-candidate QuickCraft across all four armor slots converts the unchanged
            // component-rich stack once lazily for the synchronous distribution pass.
            menu.carried = Some(rich_item("minecraft:stone", 4));
            menu.click(&env, -999, 0, Input::QuickCraft).unwrap();
            for slot in 5..9 {
                menu.click(&env, slot, 1, Input::QuickCraft).unwrap();
            }
            take_equip_conversions();
            asked.store(0, Ordering::Relaxed);
            menu.click(&env, -999, 2, Input::QuickCraft).unwrap();
            assert_eq!(
                take_equip_conversions(),
                1,
                "four armor-slot candidates in one QuickCraft pass convert the stack once"
            );
            assert_eq!(asked.load(Ordering::Relaxed), 4);
            for slot in 5..9 {
                assert_eq!(menu.slot(slot).map(|s| s.count), Some(1));
            }
            assert!(menu.carried.is_none());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn missing_and_despawned_entities_refuse_but_no_listener_keeps_data() {
        use sunrise_client::mobs::Mob;
        with_cobbler(|game, asked| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let at = sunrise_core::geometry::Vec3::new(6.5, 64.0, 5.5);
            let index = game.mobs.len();
            game.mobs.push(Mob::new(21, kind.clone(), at, 0.0));
            let prepared = game.equip_stack(&rich_helmet());
            assert!(game.can_equip(&prepared, "feet", 21, "minecraft:zombie", false));
            let before = asked.load(Ordering::Relaxed);
            assert!(!game.can_equip(&prepared, "feet", 9999, "minecraft:zombie", true));
            game.mobs.remove(index);
            assert!(!game.can_equip(&prepared, "feet", 21, "minecraft:zombie", true));
            assert_eq!(asked.load(Ordering::Relaxed), before);
            game.mobs.push(Mob::new(21, kind, at, 0.0));
            assert!(game.can_equip(&prepared, "feet", 21, "minecraft:zombie", false));
            assert_eq!(asked.load(Ordering::Relaxed), before + 1);
            take_equip_conversions();
            let other = ItemStack::new("minecraft:diamond_helmet", 3);
            let data = game.equip_stack(&other);
            assert_eq!(take_equip_conversions(), 0);
            assert!(game.can_equip(&data, "head", 9999, "minecraft:zombie", true));
        });
    }
}

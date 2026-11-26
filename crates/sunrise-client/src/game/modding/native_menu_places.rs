//! Mod menu slots backed by storage that is not a block entity (arismoko/horizon#3385):
//! NeoForge 26.3's `StackCopySlot` and `ResourceHandlerSlot`. A mod opens its menu with
//! `PlayerRef::open_mod_menu` (#3384) and backs its container slots with
//! `MenuRef::back_slots`. The guest's `native_menu` then mirrors those places, so the
//! client sees ordinary synced slots, and a pickup or shift-click on the menu goes through the
//! places' own handlers in one transaction. Nothing runs per tick: the mirror is refreshed after
//! each click on the menu and at each change notice (an `ItemPlace` write to the backing
//! player's places, queued as `NativeMenuEvent::PlacesChanged` and taken after the write), and
//! only slots whose stack changed are written. A menu no mod backed never reaches this module's
//! work: `GuestMenus::slot_places` stays `None` and every path below returns first.
use super::Game;
use super::api_values::TryIntoEngine;
use super::native_menus::native_menu_inventory_slot;
use mod_api::__private::game::*;
use mod_api::{
    Hand, ItemPlace, ItemResource, MenuContainer, MenuRef, MenuSlotBackRefusal, MenuSlotPlace,
    ServerHost,
};
use sunrise_client::items::ItemStack;
use sunrise_client::lan::{
    guest::Guest,
    menu::{NativeMenuEvent, Slots},
};
use sunrise_client::menu::{SlotContext, SlotRules, apply_menu_slot_rules, item_max_stack};

/// The places backing one guest's open mod menu.
#[derive(Clone)]
pub(in crate::game) struct MenuSlotPlaces {
    /// The menu's window id: the places end with that window.
    window: i32,
    /// What container slot `n` shows; slots past these stay the mods' own.
    places: Vec<MenuSlotPlace>,
    /// The hotbar slot holding a main-hand `HeldItem` place's stack, which takes no click.
    locked: Option<usize>,
    /// The stacks the `HeldItem` places were bound to when the slots were backed, by hand (main,
    /// off): the inventory slot holding it and the stack without its contents
    /// ([`held_identity`]). Another stack there, or another hotbar slot selected, closes the menu.
    held: [Option<(usize, ItemStack)>; 2],
    /// A change notice is queued; further writes queue none until it is taken.
    noticed: bool,
}

/// A held container's identity: the stack without the contents its `ITEM_ITEMS` handler
/// writes (`minecraft:container`, `minecraft:bundle_contents`), so writes through the menu
/// keep it and another stack put in its place does not.
fn held_identity(stack: &ItemStack) -> ItemStack {
    let mut stack = stack.clone();
    if let Some(data) = stack.extra.as_deref_mut() {
        data.container.clear();
        data.bundle_contents.clear();
        data.set_other("minecraft:bundle_contents", None);
    }
    stack.tidy();
    stack
}

impl Game<'_> {
    /// The inventory slot holding `hand`'s stack (the selected hotbar slot, or the offhand's 40) and
    /// that stack, of `guest`: read from the game in their own turn, when their record holds the
    /// outer player's.
    pub(super) fn held_by<'a>(
        &'a self,
        guest: &'a Guest,
        hand: Hand,
    ) -> (usize, Option<&'a ItemStack>) {
        let (selected, inventory, offhand) = match self.lan_acting == Some(guest.conn) {
            true => (self.selected, &self.inventory, &self.offhand),
            false => (guest.selected, &guest.inventory, &guest.offhand),
        };
        match hand {
            Hand::Off => (40, offhand.as_ref()),
            _ => (selected, inventory.get(selected).and_then(Option::as_ref)),
        }
    }

    /// `menu_back_slots`: backs `menu`'s first container slots with `places`. An
    /// empty list takes the backing away; a slot no longer backed is emptied, so the copy it
    /// showed never becomes the menu's own storage. In the player's own turn (an item's use
    /// that opened the menu), the record holds the outer player's inventory, so mirroring
    /// and resending wait until after the turn.
    pub(super) fn menu_back_slots(
        &mut self,
        menu: MenuRef,
        places: Vec<MenuSlotPlace>,
    ) -> Result<(), MenuSlotBackRefusal> {
        let player = menu.fields().player;
        let window = menu.fields().window;
        let uuid = player.uuid();
        let guest = self
            .native_menu_guest(menu)
            .ok_or(MenuSlotBackRefusal::NoPlayerOrWindow)?;
        let Some(open) = guest.menu.as_ref() else {
            return Err(MenuSlotBackRefusal::UnsupportedMenu);
        };
        // Only a blockless mod menu's slots are the game's to show (`Slots::Native`); a block's
        // or an entity's menu reads its own storage, which a backing would not replace.
        if open.slots != Slots::Native || open.owned_slots().is_some() {
            return Err(MenuSlotBackRefusal::UnsupportedMenu);
        }
        // The stacks the held places are bound to, as they are now.
        let mut held = [None, None];
        for place in &places {
            if let MenuSlotPlace::HeldItem { hand, .. } = *place {
                let (index, stack) = self.held_by(guest, hand);
                held[usize::from(hand == Hand::Off)] = stack.map(|s| (index, held_identity(s)));
            }
        }
        let selected = self.held_by(guest, Hand::Main).0;
        let conn = guest.conn;
        let size = open.size;
        let acting = self.lan_acting == Some(conn);
        if places.len() > size {
            return Err(MenuSlotBackRefusal::TooManyPlaces);
        }
        let host: &mut dyn ServerHost = self;
        if let Some(index) = places
            .iter()
            .position(|place| !place.resolves(host, player))
        {
            return Err(MenuSlotBackRefusal::PlaceUnresolved { index });
        }
        let locked = places
            .iter()
            .any(|place| {
                matches!(
                    place,
                    MenuSlotPlace::HeldItem {
                        hand: Hand::Main,
                        ..
                    }
                )
            })
            .then_some(selected);
        let transactional = !self.transactions.is_empty();
        if transactional {
            self.journal_menus(player);
            self.journal_menu_click(conn);
        }
        let Some(lan) = self.lan.as_mut() else {
            return Err(MenuSlotBackRefusal::NoPlayerOrWindow);
        };
        let count = places.len();
        let entry = &mut lan.menus.entry(conn).or_default().slot_places;
        // The slots this window's old backing mirrored that the new one does not show.
        let before = entry
            .as_ref()
            .filter(|old| old.window == window)
            .map_or(0, |old| old.places.len());
        *entry = (count > 0).then(|| MenuSlotPlaces {
            window,
            places,
            locked,
            held,
            // A notice belongs to the window, not the backing that happened to queue it.
            noticed: lan.hub.native_menu_events.iter().any(|event| {
                matches!(event, NativeMenuEvent::PlacesChanged { uuid: owner, window: queued }
                    if *owner == uuid && *queued == window)
            }),
        });
        if let Some(index) = lan.hub.guests.connection_index(conn)
            && let Some(shown) = lan.hub.guests.native_menu_mut(index)
        {
            for slot in shown.iter_mut().take(before).skip(count) {
                *slot = None;
            }
        }
        if transactional && !acting {
            self.after_commit(super::transactions::After::BackedMenuClick(
                conn, uuid, menu, window,
            ));
            return Ok(());
        }
        if acting {
            // Slots cleared above reach the client through the container sync after the turn.
            self.after_commit(super::transactions::After::MenuPlacesNotice(
                conn, uuid, window,
            ));
            return Ok(());
        }
        if count > 0 {
            self.refresh_menu_places(conn, menu);
        }
        if before > count
            && let Some(lan) = self.lan.as_mut()
        {
            lan.hub.resend_menu(conn, window, &self.world);
        }
        Ok(())
    }

    /// What container slot `n` of `conn`'s menu `window` shows, when a mod backed it.
    pub(super) fn native_menu_place(
        &self,
        conn: u32,
        window: i32,
        n: usize,
    ) -> Option<MenuSlotPlace> {
        let backed = self.active_menu_places(conn)?;
        (backed.window == window).then_some(())?;
        backed.places.get(n).copied()
    }

    /// The backing of `conn`'s menu: its window and number of slots.
    pub(super) fn menu_places_of(&self, conn: u32) -> Option<(i32, usize, Option<usize>)> {
        let backed = self.active_menu_places(conn)?;
        Some((backed.window, backed.places.len(), backed.locked))
    }

    fn active_menu_places(&self, conn: u32) -> Option<&MenuSlotPlaces> {
        let lan = self.lan.as_ref()?;
        let backed = lan.menus.get(&conn)?.slot_places.as_ref()?;
        let open = lan.hub.guest(conn)?.menu.as_ref()?;
        (open.window == backed.window && open.slots == Slots::Native).then_some(backed)
    }

    pub(super) fn forget_menu_places(&mut self, conn: u32) {
        if let Some(menus) = self.lan.as_mut().and_then(|lan| lan.menus.get_mut(&conn)) {
            menus.slot_places = None;
        }
    }

    /// Whether the stacks `conn`'s held places were bound to are still where they were: the same
    /// inventory slot (the same hotbar slot selected) holding the same stack, its contents aside.
    pub(super) fn held_places_bound(&self, conn: u32, player: mod_api::PlayerRef) -> bool {
        let Some(backed) = self
            .lan
            .as_ref()
            .and_then(|lan| lan.menus.get(&conn)?.slot_places.as_ref())
        else {
            return true;
        };
        if backed.held.iter().all(Option::is_none) {
            return true;
        }
        let Some(guest) = self.online_player_record(player) else {
            return false;
        };
        [Hand::Main, Hand::Off]
            .into_iter()
            .zip(&backed.held)
            .all(|(hand, bound)| {
                let Some((index, identity)) = bound else {
                    return true;
                };
                let (now, stack) = self.held_by(guest, hand);
                now == *index && stack.is_some_and(|stack| held_identity(stack) == *identity)
            })
    }

    /// Copies each place's stack into the guest's `native_menu` where it changed, which the next
    /// container sync sends; closes the menu, as a gone block entity's, once a place stops
    /// resolving or a held place's stack is no longer the bound one. False when it closed or was
    /// not open.
    fn refresh_menu_places(&mut self, conn: u32, menu: MenuRef) -> bool {
        let Some((window, count, _)) = self.menu_places_of(conn) else {
            return false;
        };
        let player = menu.fields().player;
        let mut open = self.held_places_bound(conn, player);
        for n in 0..count {
            let Some(place) = self.native_menu_place(conn, window, n) else {
                break;
            };
            let host: &mut dyn ServerHost = self;
            if !open || !place.resolves(host, player) {
                open = false;
                break;
            }
            let stack = ItemPlace::MenuSlot { menu, slot: n }
                .with_stack(host, |stack| stack.project())
                .flatten()
                .and_then(|stack| stack.try_into_engine(self.items()).ok());
            let Some(lan) = self.lan.as_mut() else {
                return false;
            };
            let guests = &mut lan.hub.guests;
            let Some(index) = guests.connection_index(conn) else {
                return false;
            };
            if let Some(shown) = guests.native_menu_mut(index).and_then(|m| m.get_mut(n))
                && *shown != stack
            {
                *shown = stack;
            }
        }
        if !open {
            self.forget_menu_places(conn);
            self.request_menu_close(conn, window, menu.fields().generation);
        }
        open
    }

    /// The storage-change notice: an `ItemPlace` write to `place`, if it is a place of a player
    /// whose open menu a mod backed (their hand or inventory, a container held there, their
    /// menu's slots), queues one look at that menu's places for after the write
    /// (`NativeMenuEvent::PlacesChanged`). Any other write, and every write while no menu is
    /// backed, returns at the first checks.
    pub(super) fn notice_menu_place_write(&mut self, place: &ItemPlace) {
        let mut root = place;
        while let ItemPlace::Nested { parent, .. } = root {
            root = &**parent;
        }
        let uuid = match root {
            ItemPlace::PlayerHand { player, .. } | ItemPlace::PlayerSlot { player, .. } => *player,
            ItemPlace::MenuSlot { menu, .. } => menu.fields().player.uuid(),
            _ => return,
        };
        if self
            .lan
            .as_ref()
            .is_none_or(|lan| lan.menus.values().all(|m| m.slot_places.is_none()))
        {
            return;
        }
        if let Some(conn) = self.native_conn(uuid)
            && let Some(window) = self.menu_places_of(conn).map(|(window, ..)| window)
        {
            self.after_commit(super::transactions::After::MenuPlacesNotice(
                conn, uuid, window,
            ));
        }
    }

    /// Queues one look at the places of `conn`'s backed menu (`NativeMenuEvent::PlacesChanged`),
    /// unless one is already waiting to be taken.
    pub(super) fn queue_menu_places_notice(&mut self, conn: u32, uuid: u128, window: i32) {
        let Some(lan) = self.lan.as_mut() else {
            return;
        };
        let Some(backed) = lan
            .menus
            .get_mut(&conn)
            .and_then(|m| m.slot_places.as_mut())
            .filter(|backed| backed.window == window && !backed.noticed)
        else {
            return;
        };
        backed.noticed = true;
        lan.hub
            .native_menu_events
            .push(NativeMenuEvent::PlacesChanged { uuid, window });
    }

    /// A change notice taken (`NativeMenuEvent::PlacesChanged`): the backed menu's places looked
    /// at again, or forgotten once that menu is no longer open.
    pub(super) fn menu_places_changed(&mut self, uuid: u128, window: i32) {
        let Some(conn) = self.native_conn(uuid) else {
            return;
        };
        let Some(backed) = self
            .lan
            .as_mut()
            .and_then(|lan| lan.menus.get_mut(&conn)?.slot_places.as_mut())
            .filter(|backed| backed.window == window)
        else {
            return;
        };
        backed.noticed = false;
        match self.native_menu_ref(uuid, window) {
            Some(menu) => {
                self.refresh_menu_places(conn, menu);
            }
            None => self.forget_menu_places(conn),
        }
    }

    /// The menu `window` closed (`NativeMenuEvent::Closed`): its places end with it. Its
    /// mirrored slots went with the menu (`native_menu` is cleared on close).
    pub(super) fn menu_places_closed(&mut self, uuid: u128, window: i32) {
        if let Some(conn) = self.native_conn(uuid)
            && self
                .lan
                .as_ref()
                .and_then(|lan| lan.menus.get(&conn)?.slot_places.as_ref())
                .is_some_and(|backed| backed.window == window)
        {
            self.forget_menu_places(conn);
        }
    }

    /// The inventory slot of a held backpack takes no click.
    pub(super) fn backed_menu_slot_enabled(&self, guest: &Guest, slot: usize) -> bool {
        let Some(open) = &guest.menu else {
            return true;
        };
        let locked = self
            .lan
            .as_ref()
            .and_then(|host| host.menus.get(&guest.conn)?.slot_places.as_ref())
            .filter(|backed| backed.window == open.window && open.slots == Slots::Native)
            .and_then(|backed| backed.locked);
        !locked.is_some_and(|locked| {
            i32::try_from(slot)
                .ok()
                .and_then(|slot| native_menu_inventory_slot(open.size, slot))
                == Some(locked)
        })
    }

    /// What the menu's own slot rules (`menu_slot_rules`, the locked held-backpack slot) say of a
    /// backed slot: whether it may take `amount` of `resource`, whether it may give its stack up,
    /// and how many it holds. Click admission and the slot probes both ask this, so a probe
    /// answers what a click would do.
    pub(super) fn place_menu_rules(
        &self,
        conn: u32,
        size: usize,
        locked: Option<usize>,
        slot: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> (bool, bool, u32) {
        if locked.is_some() && native_menu_inventory_slot(size, slot as i32) == locked {
            return (false, false, 0);
        }
        let Some(guest) = self.lan.as_ref().and_then(|lan| lan.hub.guest(conn)) else {
            return (false, false, 0);
        };
        let Some(open) = guest.menu.as_ref() else {
            return (false, false, 0);
        };
        if !self.listens(&open.registry_id) {
            return (true, true, u32::MAX);
        }
        let Some(stack) = resource
            .to_stack(amount)
            .and_then(|stack| stack.try_into_engine(self.items()).ok())
        else {
            return (true, true, u32::MAX);
        };
        apply_menu_slot_rules(
            Some(self),
            &open.registry_id,
            native_menu_inventory_slot(size, slot as i32).unwrap_or(slot),
            &stack,
            SlotContext::blockless(
                guest,
                open.pos.map(|(x, y, z)| [x, y, z]),
                if slot < size {
                    MenuContainer::Main
                } else {
                    MenuContainer::Player
                },
            ),
            (true, true, u32::MAX),
            item_max_stack(self.items(), &stack),
        )
    }

    /// A click on a backed menu (`AbstractContainerMenu.clicked`'s `PICKUP` and `QUICK_MOVE`),
    /// done through the places' handlers in one transaction. `None` lets ordinary display-menu
    /// routing run, for a menu no mod backed or a click this backing does not handle. Before any
    /// click on a backed menu a held place's stack must still be the bound one; else the menu
    /// closes and the click does nothing.
    pub(super) fn place_menu_click(
        &mut self,
        uuid: u128,
        window: i32,
        slot: i32,
        button: i32,
        mode: i32,
    ) -> Option<bool> {
        use super::lan_menus::ClickReply;
        let conn = self.native_conn(uuid)?;
        let (backed_window, count, locked) = self.menu_places_of(conn)?;
        if backed_window != window {
            return None;
        }
        let menu = self.native_menu_ref(uuid, window)?;
        if !self.held_places_bound(conn, menu.fields().player) {
            self.reply_menu_click(conn, ClickReply::Close(window));
            return Some(true);
        }
        let at = usize::try_from(slot).ok()?;
        let guest = self.native_menu_guest(menu)?;
        let open = guest.menu.as_ref()?;
        let (size, total) = (open.size, open.slot_count());
        let pickup = mode == 0 && matches!(button, 0 | 1);
        if !(pickup || mode == 1) || (count..size).contains(&at) || at >= total {
            return None;
        }
        let inventory = native_menu_inventory_slot(size, slot);
        // The held backpack's own slot takes no click, so it can't go into itself.
        let applied = if at < count
            && (ItemPlace::MenuSlot { menu, slot: at }).amount(self) > 0
            && menu.probe_slot_gives(self, at) != Ok(Some(true))
        {
            true
        } else if inventory.is_none() || inventory != locked {
            let accepts = |game: &Game<'_>, slot, resource: &ItemResource, amount| {
                game.place_menu_rules(conn, size, locked, slot, resource, amount)
            };
            if pickup {
                click_pickup(self, menu, at, button == 1, accepts)
            } else {
                click_quick_move(self, menu, at, size, total - size, count, accepts)
            }
        } else {
            true
        };
        if applied {
            self.after_commit(super::transactions::After::BackedMenuClick(
                conn, uuid, menu, window,
            ));
        } else {
            self.reply_menu_click(conn, ClickReply::Resend(window));
        }
        Some(applied)
    }

    pub(super) fn publish_backed_menu_click(
        &mut self,
        conn: u32,
        uuid: u128,
        menu: MenuRef,
        window: i32,
    ) {
        if self.native_menu_ref(uuid, window) == Some(menu)
            && (self.menu_places_of(conn).is_none() || self.refresh_menu_places(conn, menu))
        {
            self.reply_menu_click(conn, super::lan_menus::ClickReply::Resend(window));
        }
    }
}

/// `PICKUP` on menu slot `at`: an empty cursor takes all (or half, `right`) of the slot; a
/// full one puts all (or one) in, or swaps with a slot of another item. All or nothing.
fn click_pickup(
    game: &mut Game<'_>,
    menu: MenuRef,
    slot: usize,
    right: bool,
    accepts: impl Fn(&Game<'_>, usize, &ItemResource, u32) -> (bool, bool, u32),
) -> bool {
    let cursor = menu.cursor();
    let at = ItemPlace::MenuSlot { menu, slot };
    game.storage_transaction(|host, tx| {
        let here = at.resource(host);
        let have = at.amount(host);
        if let Some(here) = &here
            && !accepts(host, slot, here, have).1
        {
            return Ok(());
        }
        let Some(held) = cursor.resource(host) else {
            let Some(here) = here else {
                return Ok(());
            };
            let want = if right { have.div_ceil(2) } else { have };
            let out = at.extract(host, tx, &here, want).map_err(|_| ())?;
            return match cursor.insert(host, tx, &here, out) == Ok(out) {
                true => Ok(()),
                false => Err(()),
            };
        };
        let carried = cursor.amount(host);
        let (place, _, max) = accepts(host, slot, &held, carried);
        if !place {
            return Ok(());
        }
        let room = if here.as_ref() == Some(&held) {
            max.saturating_sub(have)
        } else {
            max
        };
        let put = at
            .insert(host, tx, &held, (if right { 1 } else { carried }).min(room))
            .map_err(|_| ())?;
        if put > 0 {
            return match cursor.extract(host, tx, &held, put) == Ok(put) {
                true => Ok(()),
                false => Err(()),
            };
        }
        let Some(here) = here.filter(|here| *here != held && carried <= max) else {
            return Ok(());
        };
        let swapped = at.extract(host, tx, &here, have) == Ok(have)
            && cursor.extract(host, tx, &held, carried) == Ok(carried)
            && at.insert(host, tx, &held, carried) == Ok(carried)
            && cursor.insert(host, tx, &here, have) == Ok(have);
        match swapped {
            true => Ok(()),
            false => Err(()),
        }
    })
    .is_ok()
}

/// `QUICK_MOVE` on menu slot `at` of a menu of `size` container slots, `count` of them backed
/// (`ChestMenu.quickMoveStack`): a container slot's stack goes to the inventory, hotbar end
/// first; an inventory slot's into the backed slots in order, as allowed by the menu's slot rules.
/// Onto matching stacks first, then into empty slots (`moveItemStackTo`), all or
/// nothing.
fn click_quick_move(
    game: &mut Game<'_>,
    menu: MenuRef,
    at: usize,
    size: usize,
    player_slots: usize,
    count: usize,
    accepts: impl Fn(&Game<'_>, usize, &ItemResource, u32) -> (bool, bool, u32),
) -> bool {
    let from = ItemPlace::MenuSlot { menu, slot: at };
    let Some(resource) = from.resource(game) else {
        return true;
    };
    let amount = from.amount(game);
    if !accepts(game, at, &resource, amount).1 {
        return true;
    }
    let (first, len, reverse) = match at < size {
        true => (size, player_slots, true),
        false => (0, count, false),
    };
    game.storage_transaction(|host, tx| {
        let mut left = amount;
        for merge in [true, false] {
            for i in 0..len {
                if left == 0 {
                    return Ok(());
                }
                let slot = if reverse {
                    first + len - 1 - i
                } else {
                    first + i
                };
                let to = ItemPlace::MenuSlot { menu, slot };
                match to.resource(host) {
                    Some(here) if merge && here == resource => {}
                    None if !merge => {}
                    _ => continue,
                }
                let (place, _, max) = accepts(host, slot, &resource, left);
                if !place {
                    continue;
                }
                let room = max.saturating_sub(to.amount(host));
                let moved = to
                    .insert(host, tx, &resource, left.min(room))
                    .map_err(|_| ())?;
                if moved > 0 {
                    if from.extract(host, tx, &resource, moved) != Ok(moved) {
                        return Err(());
                    }
                    left -= moved;
                }
            }
        }
        Ok(())
    })
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game as with_base_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use super::*;
    use mod_api::{HostTransaction, MenuReplace, MenuSlotBacking};
    use sunrise_client::items::ItemStack;
    use sunrise_client::lan::menu::Slots;

    struct Backpack;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Backpack {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let item = r.item_key("minecraft:shulker_box");
            r.item_container(&item, |_: &dyn mod_api::Host| Some(27));
        }
    }

    fn with_game(test: impl FnOnce(&mut Game<'_>)) {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "backpack_fixture",
            Box::new(Backpack),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || with_base_game(test));
    }

    /// Guest 1 (UUID 123) holding a shulker box in hotbar slot 0 with 64 stone in inventory
    /// slot 9.
    fn backpack_guest(game: &mut Game<'_>) {
        open_to_lan(game);
        let mut player = guest(1);
        player.selected = 0;
        player.inventory[0] = Some(ItemStack::new("minecraft:shulker_box", 1));
        player.inventory[9] = Some(ItemStack::new("minecraft:stone", 64));
        game.lan.as_mut().unwrap().hub.guests.push(player);
    }

    /// A 27-slot mod chest menu opened for guest 1: its handle.
    fn open_pack(game: &mut Game<'_>) -> MenuRef {
        let player = game.player(123).unwrap();
        let host: &mut dyn ServerHost = game;
        player
            .open_chest_menu(
                host,
                3,
                mod_api::Text::literal("pack"),
                Vec::new(),
                MenuReplace::CloseFirst,
            )
            .unwrap()
    }

    /// [`backpack_guest`] with the menu open: its handle.
    fn backpack_menu(game: &mut Game<'_>) -> MenuRef {
        backpack_guest(game);
        open_pack(game)
    }

    fn held_slot(n: usize) -> ItemPlace {
        ItemPlace::Nested {
            parent: Box::new(ItemPlace::PlayerHand {
                player: 123,
                hand: Hand::Main,
            }),
            slot: n,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_shift_click_into_a_held_backpack_writes_through_its_handler() {
        with_game(|game| {
            let stale = backpack_menu(game);
            let menu = open_pack(game);
            let window = menu.container_id(game).unwrap();
            let host: &mut dyn ServerHost = game;
            // Unbacked, the menu is the mod's own, as before.
            assert_eq!(menu.slot_backing(host, 0), Some(MenuSlotBacking::Native(0)));
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), None);

            let places: Vec<_> = (0..27)
                .map(|slot| MenuSlotPlace::HeldItem {
                    hand: Hand::Main,
                    slot,
                })
                .collect();
            assert_eq!(
                stale.back_slots(game, places.clone()),
                Err(MenuSlotBackRefusal::NoPlayerOrWindow)
            );
            assert_eq!(menu.back_slots(game, places), Ok(()));
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                menu.slot_backing(host, 0),
                Some(MenuSlotBacking::Place(MenuSlotPlace::HeldItem {
                    hand: Hand::Main,
                    slot: 0
                }))
            );

            // Inventory slot 9 is menu slot 27: a shift-click moves the stone into the box.
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), Some(true));
            let host: &mut dyn ServerHost = game;
            assert_eq!(held_slot(0).amount(host), 64);
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 27 }.with_stack(host, |_| ()),
                None
            );
            assert_eq!(ItemPlace::MenuSlot { menu, slot: 0 }.amount(host), 64);
            let stone = ItemPlace::MenuSlot { menu, slot: 0 }
                .resource(host)
                .unwrap();
            assert_eq!(menu.slot_gives(host, 0), None);
            assert_eq!(menu.probe_slot_gives(host, 0), Ok(Some(true)));
            assert_eq!(held_slot(0).amount(host), 64, "the pickup probe rolls back");
            assert_eq!(menu.slot_holds(host, 54, stone.as_stack()), Some(0));
            assert_eq!(menu.slot_gives(host, 54), Some(false));
            assert_eq!(
                menu.probe_slot_holds(host, 54, stone.as_stack()),
                Ok(Some(0))
            );
            assert_eq!(menu.probe_slot_gives(host, 54), Ok(Some(false)));

            // The box's own hotbar slot (menu slot 54) takes no click.
            assert_eq!(game.place_menu_click(123, window, 54, 0, 0), Some(true));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.cursor().with_stack(host, |_| ()), None);
            let held = ItemPlace::PlayerHand {
                player: 123,
                hand: Hand::Main,
            };
            assert_eq!(held.amount(host), 1, "the box stays in the hand");

            // The box gone from the hand, the write's notice closes the menu once taken.
            let resource = held.resource(host).unwrap();
            let taken = host.transaction(|host, tx| held.extract(host, tx, &resource, 1));
            assert_eq!(taken, Ok(1));
            game.run_native_menu_events();
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), None);
        });
    }

    /// The guest's record, to change their hands as a packet or a command would.
    fn edit_guest(game: &mut Game<'_>, edit: impl FnOnce(&mut sunrise_client::lan::guest::Guest)) {
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        let index = guests.connection_index(1).unwrap();
        guests.update(index, edit);
    }

    fn held_places(hand: Hand, slots: std::ops::Range<usize>) -> Vec<MenuSlotPlace> {
        slots
            .map(|slot| MenuSlotPlace::HeldItem { hand, slot })
            .collect()
    }

    /// Stone in the box, inventory slot 9 and the menu's own slots: un-backing keeps it at 64.
    fn stone(game: &mut Game<'_>, menu: MenuRef) -> u32 {
        let host: &mut dyn ServerHost = game;
        let boxed: u32 = (0..27).map(|n| held_slot(n).amount(host)).sum();
        let own: u32 = (0..27)
            .map(|slot| match menu.slot_backing(host, slot) {
                // A backed slot is a view of the box, already counted above.
                Some(MenuSlotBacking::Native(_)) => ItemPlace::MenuSlot { menu, slot }.amount(host),
                _ => 0,
            })
            .sum();
        let carried = ItemPlace::PlayerSlot {
            player: 123,
            slot: 9,
        }
        .amount(host);
        boxed + own + carried
    }

    #[test]
    #[ignore = "daily"]
    fn un_backing_or_shortening_empties_the_mirrored_slots_and_keeps_the_items_once() {
        with_game(|game| {
            let menu = backpack_menu(game);
            let window = menu.container_id(game).unwrap();
            let places = held_places(Hand::Main, 0..27);
            assert_eq!(menu.back_slots(game, places), Ok(()));
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), Some(true));
            assert_eq!(stone(game, menu), 64);
            // Shortened to slot 1 of the box: menu slot 0 shows that empty slot, the rest nothing.
            let places = held_places(Hand::Main, 1..2);
            assert_eq!(menu.back_slots(game, places), Ok(()));
            let host: &mut dyn ServerHost = game;
            assert_eq!(ItemPlace::MenuSlot { menu, slot: 0 }.amount(host), 0);
            assert_eq!(held_slot(0).amount(host), 64);
            assert_eq!(stone(game, menu), 64);
            // Un-backed: the menu's own slots hold no copy to take out.
            assert_eq!(menu.back_slots(game, Vec::new()), Ok(()));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.slot_backing(host, 0), Some(MenuSlotBacking::Native(0)));
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 0 }.with_stack(host, |_| ()),
                None
            );
            assert_eq!(stone(game, menu), 64);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_backing_names_why_it_refuses() {
        with_game(|game| {
            let menu = backpack_menu(game);
            let back = |game: &mut Game<'_>, places| menu.back_slots(game, places);
            assert_eq!(
                back(game, held_places(Hand::Main, 0..28)),
                Err(MenuSlotBackRefusal::TooManyPlaces)
            );
            // Nothing in the offhand: the second place names no storage.
            let mut places = held_places(Hand::Main, 0..1);
            places.extend(held_places(Hand::Off, 0..1));
            assert_eq!(
                back(game, places),
                Err(MenuSlotBackRefusal::PlaceUnresolved { index: 1 })
            );
            assert_eq!(back(game, held_places(Hand::Main, 0..1)), Ok(()));
            let host: &mut dyn ServerHost = game;
            assert!(matches!(
                menu.slot_backing(host, 0),
                Some(MenuSlotBacking::Place(_))
            ));
            assert_eq!(
                game.as_guest(1, |game| back(game, held_places(Hand::Main, 0..1))),
                Some(Ok(()))
            );
            // A block's menu cannot use a backing, even with a leftover record for the window.
            edit_guest(game, |g| {
                g.menu.as_mut().unwrap().slots = Slots::Block((0, 64, 0));
            });
            assert_eq!(
                back(game, held_places(Hand::Main, 0..1)),
                Err(MenuSlotBackRefusal::UnsupportedMenu)
            );
            assert_eq!(game.menu_places_of(1), None);
            edit_guest(game, |g| g.menu.as_mut().unwrap().slots = Slots::Native);
            assert_eq!(back(game, held_places(Hand::Main, 0..1)), Ok(()));
            assert_eq!(
                game.as_guest(1, |game| back(game, held_places(Hand::Main, 0..1))),
                Some(Ok(()))
            );
            // A closed/stale menu handle is refused, in its own turn too.
            let stale_menu = menu;
            let _fresh = open_pack(game);
            let refused = Err(MenuSlotBackRefusal::NoPlayerOrWindow);
            let stale =
                |game: &mut Game<'_>| stale_menu.back_slots(game, held_places(Hand::Main, 0..1));
            assert_eq!(stale(game), refused);
            assert_eq!(game.as_guest(1, stale), Some(refused));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_write_into_the_box_is_mirrored_at_its_notice() {
        with_game(|game| {
            let menu = backpack_menu(game);
            let places = held_places(Hand::Main, 0..27);
            assert_eq!(menu.back_slots(game, places), Ok(()));
            let host: &mut dyn ServerHost = game;
            let custom_data = mod_api::ComponentId::new("minecraft:custom_data").unwrap();
            let data = mod_api::Tag::Compound(
                [("value".into(), mod_api::Tag::Int(42))]
                    .into_iter()
                    .collect(),
            );
            let stone = ItemPlace::PlayerSlot {
                player: 123,
                slot: 9,
            }
            .resource(host)
            .unwrap()
            .with_component(host, &custom_data, data.clone())
            .unwrap();
            let put = host.transaction(|host, tx| held_slot(4).insert(host, tx, &stone, 5));
            assert_eq!(put, Ok(5));
            let shown = |game: &Game<'_>| {
                let guest = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
                guest.native_menu[4].as_ref().map(|s| s.count)
            };
            assert_eq!(shown(game), None, "mirrored after the write, not inside it");
            let place = ItemPlace::MenuSlot { menu, slot: 4 };
            let found = place.with_stack(game, |stack| (stack.count(), stack.project()));
            let (count, projected) = found.unwrap();
            assert_eq!(count, 5);
            let projected = projected.unwrap();
            assert_eq!(projected.count(), 5);
            assert_eq!(
                mod_api::ItemResource::of(&projected).component(&custom_data),
                Some(data)
            );
            game.run_native_menu_events();
            assert_eq!(shown(game), Some(5));

            let host: &mut dyn ServerHost = game;
            let taken = host.transaction(|host, tx| held_slot(4).extract(host, tx, &stone, 5));
            assert_eq!(taken, Ok(5));
            assert_eq!(shown(game), Some(5), "the mirror has not caught up yet");
            assert_eq!(
                place.with_stack(game, |_| ()),
                None,
                "no fallback to the display mirror"
            );
        });
    }

    /// An item's use opens its menu and backs it at once, inside the player's own turn.
    #[test]
    #[ignore = "daily"]
    fn a_menu_backed_in_the_players_own_turn_shows_the_held_box_after_it() {
        with_game(|game| {
            backpack_guest(game);
            let (menu, backed) = game
                .as_guest(1, |game| {
                    let host: &mut dyn ServerHost = game;
                    let carried = ItemPlace::PlayerSlot {
                        player: 123,
                        slot: 9,
                    };
                    let stone = carried.resource(host).unwrap();
                    let put = host.transaction(|host, tx| held_slot(4).insert(host, tx, &stone, 5));
                    assert_eq!(put, Ok(5));
                    let menu = open_pack(game);
                    let places = held_places(Hand::Main, 0..27);
                    (menu, menu.back_slots(game, places))
                })
                .unwrap();
            assert_eq!(backed, Ok(()));
            let window = menu.container_id(game).unwrap();
            let shown = |game: &Game<'_>| {
                let guest = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
                guest.native_menu[4].as_ref().map(|s| s.count)
            };
            assert_eq!(shown(game), None, "mirrored after the turn, not inside it");
            game.run_native_menu_events();
            assert_eq!(shown(game), Some(5));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), Some(window));

            // The box bound is the one the player held in that turn: another box selected, the
            // next click closes the menu and moves nothing.
            edit_guest(game, |g| {
                g.inventory[1] = Some(ItemStack::new("minecraft:shulker_box", 1));
                g.selected = 1;
            });
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), Some(true));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), None);
            let carried = ItemPlace::PlayerSlot {
                player: 123,
                slot: 9,
            };
            assert_eq!(carried.amount(host), 64);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn rebinding_coalesces_notices_until_the_right_window_is_processed() {
        with_game(|game| {
            backpack_guest(game);
            let (old, new) = game
                .as_guest(1, |game| {
                    let old = open_pack(game);
                    let old_window = old.container_id(game).unwrap();
                    old.back_slots(game, held_places(Hand::Main, 0..1)).unwrap();
                    let new = open_pack(game);
                    let new_window = new.container_id(game).unwrap();
                    for places in [
                        held_places(Hand::Main, 0..2),
                        Vec::new(),
                        held_places(Hand::Main, 1..2),
                    ] {
                        new.back_slots(game, places).unwrap();
                    }
                    (old_window, new_window)
                })
                .unwrap();
            // Deliver the old window's change as the event drain does, then write again.
            game.menu_places_changed(123, old);
            game.notice_menu_place_write(&held_slot(1));
            let windows: Vec<_> = game
                .lan
                .as_ref()
                .unwrap()
                .hub
                .native_menu_events
                .iter()
                .filter_map(|event| match event {
                    NativeMenuEvent::PlacesChanged { uuid: 123, window } => Some(*window),
                    _ => None,
                })
                .collect();
            assert_eq!(
                windows,
                [old, new],
                "one notice per window, even across an unback"
            );
            game.run_native_menu_events();
            game.notice_menu_place_write(&held_slot(1));
            assert!(matches!(
                game.lan.as_ref().unwrap().hub.native_menu_events.as_slice(),
                [NativeMenuEvent::PlacesChanged { uuid: 123, window }] if *window == new
            ));
        });
    }

    /// The bound stack replaced by `replace`, then a shift-click: the menu closes and the click
    /// moves nothing.
    fn replaced_closes(hand: Hand, replace: impl FnOnce(&mut sunrise_client::lan::guest::Guest)) {
        with_game(|game| {
            let menu = backpack_menu(game);
            let window = menu.container_id(game).unwrap();
            if hand == Hand::Off {
                edit_guest(game, |g| g.offhand = g.inventory[0].take());
            }
            let places = held_places(hand, 0..27);
            assert_eq!(menu.back_slots(game, places), Ok(()));
            edit_guest(game, replace);
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), Some(true));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), None);
            let carried = ItemPlace::PlayerSlot {
                player: 123,
                slot: 9,
            };
            assert_eq!(carried.amount(host), 64);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn another_stack_in_the_hand_closes_the_menu() {
        replaced_closes(Hand::Main, |g| {
            g.inventory[0] = Some(ItemStack::new("minecraft:red_shulker_box", 1));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn another_stack_in_the_offhand_closes_the_menu() {
        replaced_closes(Hand::Off, |g| {
            g.offhand = Some(ItemStack::new("minecraft:red_shulker_box", 1));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn another_hotbar_slot_selected_closes_the_menu() {
        replaced_closes(Hand::Main, |g| {
            g.inventory[1] = Some(ItemStack::new("minecraft:shulker_box", 1));
            g.selected = 1;
        });
    }

    #[test]
    #[ignore = "daily"]
    fn backed_slot_rules_ask_the_live_handler_not_the_display_copy() {
        use mod_api::{
            ItemResource, Overanswered, ResourceHandler, TransactionContext, TransactionError,
        };
        struct Handler;
        impl ResourceHandler<ItemResource> for Handler {
            fn size(&self, _: &mut dyn ServerHost) -> usize {
                2
            }
            fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
                Some(ItemResource::of(
                    &mod_api::ItemStack::new(
                        mod_api::__private::item_key("minecraft:stone").unwrap(),
                        1,
                    )
                    .unwrap(),
                ))
            }
            fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
                1
            }
            fn capacity(
                &self,
                _: &mut dyn ServerHost,
                slot: usize,
                _: Option<&ItemResource>,
            ) -> u32 {
                if slot == 0 { 7 } else { u32::MAX }
            }
            fn is_valid(&self, _: &mut dyn ServerHost, _: usize, stack: &ItemResource) -> bool {
                mod_api::__private::item_key_id(stack.as_stack().item()) == Some("minecraft:stone")
            }
            fn insert_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                _: u32,
            ) -> Result<u32, TransactionError<Overanswered>> {
                Ok(0)
            }
            fn extract_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                _: u32,
            ) -> Result<u32, TransactionError<Overanswered>> {
                Ok(0)
            }
        }
        struct ReadOnly;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ReadOnly {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.item_capability_before_game(
                    &mod_api::ITEM_ITEMS,
                    "minecraft:shulker_box",
                    |_, _| Some(Box::new(Handler)),
                );
            }
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("readonly_pack", Box::new(ReadOnly))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_base_game(|game| {
                let menu = backpack_menu(game);
                assert_eq!(menu.back_slots(game, held_places(Hand::Main, 0..2)), Ok(()));
                let stone = menu.slot(game, 0).unwrap().unwrap();
                let dirt = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:dirt").unwrap(),
                    1,
                )
                .unwrap();
                let read: &dyn mod_api::Host = game;
                assert!(matches!(
                    menu.slot_backing(read, 0),
                    Some(MenuSlotBacking::Place(_))
                ));
                assert_eq!(menu.slot_holds(read, 0, &stone), None);
                assert_eq!(menu.slot_gives(read, 0), None);
                assert_eq!(menu.probe_slot_holds(game, 0, &stone), Ok(Some(7)));
                assert_eq!(menu.probe_slot_holds(game, 0, &dirt), Ok(Some(0)));
                assert_eq!(
                    menu.probe_slot_holds(game, 1, &stone),
                    Ok(Some(i32::MAX as u32)),
                    "ResourceHandlerSlot.getMaxStackSize clamps capacity to i32::MAX"
                );
                assert_eq!(menu.probe_slot_gives(game, 0), Ok(Some(false)));
                // Hotbar zero backs the open menu, so its own inventory slot takes no move.
                let holding_slot = game
                    .native_menu_guest(menu)
                    .unwrap()
                    .menu
                    .as_ref()
                    .unwrap()
                    .size
                    + 27;
                assert_eq!(menu.slot_holds(game, holding_slot, &stone), Some(0));
                assert_eq!(menu.slot_gives(game, holding_slot), Some(false));
                assert_eq!(
                    menu.probe_slot_holds(game, holding_slot, &stone),
                    Ok(Some(0))
                );
                assert_eq!(menu.probe_slot_gives(game, holding_slot), Ok(Some(false)));
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn backed_clicks_enforce_handler_pickup_probe_before_taking_or_quick_moving() {
        use mod_api::{
            ItemResource, Overanswered, ResourceHandler, TransactionContext, TransactionError,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        };
        struct BulkOnlyHandler(Arc<AtomicU32>);
        impl ResourceHandler<ItemResource> for BulkOnlyHandler {
            fn size(&self, _: &mut dyn ServerHost) -> usize {
                1
            }
            fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
                (self.0.load(Ordering::Relaxed) > 0).then(|| {
                    ItemResource::of(
                        &mod_api::ItemStack::new(
                            mod_api::__private::item_key("minecraft:stone").unwrap(),
                            1,
                        )
                        .unwrap(),
                    )
                })
            }
            fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
                self.0.load(Ordering::Relaxed)
            }
            fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&ItemResource>) -> u32 {
                64
            }
            fn is_valid(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> bool {
                true
            }
            fn insert_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                amount: u32,
            ) -> Result<u32, TransactionError<Overanswered>> {
                Ok(amount)
            }
            fn extract_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                amount: u32,
            ) -> Result<u32, TransactionError<Overanswered>> {
                // Refuse extracting 1 so ResourceHandlerSlot.mayPickup is false,
                // while still allowing larger explicit extracts.
                if amount <= 1 {
                    return Ok(0);
                }
                let held = self.0.load(Ordering::Relaxed);
                let taken = held.min(amount);
                self.0.store(held - taken, Ordering::Relaxed);
                Ok(taken)
            }
        }
        struct BulkPack(Arc<AtomicU32>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for BulkPack {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let count = Arc::clone(&self.0);
                r.item_capability_before_game(
                    &mod_api::ITEM_ITEMS,
                    "minecraft:shulker_box",
                    move |_, _| Some(Box::new(BulkOnlyHandler(Arc::clone(&count)))),
                );
            }
        }
        let stored = Arc::new(AtomicU32::new(4));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "bulk_pack",
            Box::new(BulkPack(Arc::clone(&stored))),
        )]
        .into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_base_game(|game| {
                let menu = backpack_menu(game);
                let window = menu.container_id(game).unwrap();
                assert_eq!(menu.back_slots(game, held_places(Hand::Main, 0..1)), Ok(()));
                assert_eq!(menu.probe_slot_gives(game, 0), Ok(Some(false)));

                // Empty-cursor pickup click refuses when mayPickup is false.
                assert_eq!(game.place_menu_click(123, window, 0, 0, 0), Some(true));
                assert_eq!(stored.load(Ordering::Relaxed), 4);
                assert_eq!(menu.cursor().with_stack(game, |_| ()), None);

                // Quick-move click refuses when mayPickup is false.
                assert_eq!(game.place_menu_click(123, window, 0, 0, 1), Some(true));
                assert_eq!(stored.load(Ordering::Relaxed), 4);

                // Non-empty cursor swap click refuses when mayPickup is false.
                edit_guest(game, |guest| {
                    guest.carried = Some(ItemStack::new("minecraft:dirt", 1));
                });
                assert_eq!(game.place_menu_click(123, window, 0, 0, 0), Some(true));
                assert_eq!(stored.load(Ordering::Relaxed), 4);
                let carried_id = menu
                    .cursor()
                    .resource(game)
                    .and_then(|r| mod_api::__private::item_key_id(r.as_stack().item()));
                assert_eq!(carried_id, Some("minecraft:dirt"));

                // Explicit ItemPlace::MenuSlot transfer still reaches the handler directly.
                let slot = ItemPlace::MenuSlot { menu, slot: 0 };
                let stone = slot.resource(game).unwrap();
                let host: &mut dyn ServerHost = game;
                let moved =
                    host.transaction(|host, tx| Ok::<_, ()>(slot.extract(host, tx, &stone, 4)));
                assert_eq!(moved, Ok(Ok(4)));
                assert_eq!(stored.load(Ordering::Relaxed), 0);
            })
        });
    }

    /// A held box's menu, opened from code at the mod block whose container
    /// opens its type, with backing slot 0 display-only under the menu rules.
    struct Satchel;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Satchel {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let item = r.item_key("minecraft:shulker_box");
            r.item_container(&item, |_: &dyn mod_api::Host| Some(27));
            let block = r.block_key("satchel:stand");
            r.block_container(
                &block,
                mod_api::ModContainer {
                    slots: 9,
                    menu: Some(SATCHEL_MENU.into()),
                    ..Default::default()
                },
            );
            r.menu_part(SATCHEL_MENU, SatchelSlotRules);
        }
        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            event
                .register(mod_api::Block::new("satchel:stand", "minecraft:barrel").with_item(false))
                .unwrap();
        }
    }
    struct SatchelSlotRules;
    #[mod_api::__subscriptions]
    impl mod_api::MenuPart for SatchelSlotRules {
        fn menu_slot_rules(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MenuSlotRulesEvent<'_>,
        ) {
            if event.container == MenuContainer::Main && event.slot == 0 {
                event.may_place = false;
                event.may_pickup = false;
            }
        }
    }

    /// The content mod menu type the satchel opens: a prepared registry slot for a synthetic
    /// definition, as `native_menus`' tests use.
    const SATCHEL_MENU: &str = "satchel:menu";

    /// #1172: a menu `player_open_mod_menu` opens keeps its handle and backing, and a
    /// shift-click on the server passes over the slot its menu declares display-only.
    /// Neither pickup nor shift-click can move an item out of the display-only slot.
    #[test]
    #[ignore = "daily"]
    fn a_code_opened_backed_menu_respects_its_display_only_slot() {
        // Content registries are process-global.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menu_places::tests::a_code_opened_backed_menu_respects_its_display_only_slot",
            "SUNRISE_MENU_PLACES_HIDDEN_CHILD",
            &["hidden"],
        ) {
            return;
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("satchel", Box::new(Satchel))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let root = crate::game::mods::content_root(&base, &Default::default()).unwrap();
            sunrise_client::containers::with_mod_menu_types(
                [(
                    SATCHEL_MENU.into(),
                    "satchel".into(),
                    9,
                    0,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    true,
                    None,
                )],
                || {
                    super::super::native_entity_refs::tests::with_game_role(
                        root.prepared().to_owned(),
                        crate::game::execution_role::ExecutionRole::IntegratedServer,
                        open_back_and_shift_click,
                    )
                },
            );
        });
    }

    fn open_back_and_shift_click(game: &mut Game<'_>) {
        let pos = [1, 64, 2];
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
        );
        let stand = game
            .pack
            .assets
            .registry
            .default_state("satchel:stand")
            .unwrap();
        game.world.set(pos[0], pos[1], pos[2], stand).unwrap();
        open_to_lan(game);
        let mut player = guest(1);
        player.selected = 0;
        player.brand = "sunrise".into();
        player.sender.menu_extra_rev = 1;
        player.inventory[0] = Some(ItemStack::new("minecraft:shulker_box", 1));
        player.inventory[9] = Some(ItemStack::new("minecraft:stone", 64));
        game.lan.as_mut().unwrap().hub.guests.push(player);
        let key = super::super::native_guest_turn_tests::register_test_menu(game, SATCHEL_MENU);
        let player = game.player(123).unwrap();
        let menu = game
            .player_open_mod_menu(player, {
                let mut request = mod_api::ModMenuOpen::new(key, mod_api::Text::literal("satchel"));
                request.pos = Some(pos);
                request
            })
            .unwrap()
            .unwrap();
        let window = menu.container_id(game).unwrap();
        // The handle and the backing stay usable.
        assert_eq!(menu.back_slots(game, held_places(Hand::Main, 0..9)), Ok(()));
        // Inventory slot 9 is menu slot 9: the stone passes over display-only slot 0.
        assert_eq!(game.place_menu_click(123, window, 9, 0, 1), Some(true));
        let host: &mut dyn ServerHost = game;
        assert_eq!(held_slot(0).amount(host), 0);
        assert_eq!(held_slot(1).amount(host), 64);
        assert_eq!(ItemPlace::MenuSlot { menu, slot: 1 }.amount(host), 64);

        // The menu slot obeys its rules even when addressed through a handle.
        // The storage itself can still be stocked directly for display.
        let hidden = ItemPlace::MenuSlot { menu, slot: 0 };
        let stone = ItemPlace::MenuSlot { menu, slot: 1 }
            .resource(host)
            .unwrap();
        assert_eq!(
            host.transaction(|host, tx| hidden.insert(host, tx, &stone, 5)),
            Ok(0)
        );
        assert_eq!(
            host.transaction(|host, tx| held_slot(0).insert(host, tx, &stone, 5)),
            Ok(5)
        );
        assert_eq!(hidden.amount(host), 5);
        assert_eq!(
            host.transaction(|host, tx| hidden.extract(host, tx, &stone, 5)),
            Ok(0)
        );
        // The probes answer what a click would: the handler permits both, the menu's rules veto.
        let stone_stack =
            mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:stone").unwrap(), 1)
                .unwrap();
        assert_eq!(menu.probe_slot_holds(game, 0, &stone_stack), Ok(Some(0)));
        assert_eq!(menu.probe_slot_gives(game, 0), Ok(Some(false)));
        let contents = |game: &mut Game<'_>| {
            (0..45)
                .map(|slot| {
                    ItemPlace::MenuSlot { menu, slot }
                        .with_stack(game, |stack| stack.project())
                        .flatten()
                })
                .collect::<Vec<_>>()
        };
        let before = contents(game);
        for mode in [0, 1] {
            assert_eq!(game.place_menu_click(123, window, 0, 0, mode), Some(true));
            assert_eq!(contents(game), before);
            assert_eq!(menu.cursor().with_stack(game, |_| ()), None);
            assert_eq!(held_slot(0).amount(game), 5);
        }
    }
}

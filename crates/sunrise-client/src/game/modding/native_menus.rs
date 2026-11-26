//! Blockless server menus for native mods: vanilla chest displays or declared mod menus.
//! Owned slots run ordinary clicks; display slots move nothing by default. Menu parts can
//! replace a click, and the owning menu part receives any detached owned contents.
use super::api_values::TryIntoEngine;
use super::{Game, lan::LanHost, lan_menus::GuestMenus};
use mod_api::__private::game::*;
use mod_api::{MenuRef, MenuReplace, ModMenuOpen};
use sunrise_client::{
    containers::ModMenuType,
    items::ItemStack,
    lan::{
        data::component_nbt,
        menu::{Menu, NativeMenuEvent, Slots},
    },
    text::Text,
};
use sunrise_core::nbt::Tag;

/// The inventory slot (0..9 the hotbar, 9..36 the rest) a click on `slot`
/// of a native menu of `size` slots landed on, when it is the player's
/// inventory below the menu (`ChestMenu`: the menu's slots, then the
/// inventory's 27, then the hotbar's 9).
#[allow(dead_code)]
pub(crate) fn native_menu_inventory_slot(size: usize, slot: i32) -> Option<usize> {
    let n = usize::try_from(slot).ok()?.checked_sub(size)?;
    match n {
        0..27 => Some(n + 9),
        27..36 => Some(n - 27),
        _ => None,
    }
}

/// An open that must not replace the current menu or return its cursor until commit.
pub(super) enum PendingMenuOpen {
    Native(NativeMenuOpen),
    ModMenu(PendingModMenuOpen),
    Block(mod_api::PlayerRef, mod_api::BlockEntityRef),
    Ender(mod_api::PlayerRef, Text),
}

pub(super) struct PendingModMenuOpen {
    player: mod_api::PlayerRef,
    id: String,
    kind: &'static ModMenuType,
    title: Tag,
    extra: Vec<u8>,
    data: Vec<i32>,
    pos: Option<[i32; 3]>,
    opening_hand: Option<mod_api::Hand>,
    replace: MenuReplace,
}

pub(super) struct NativeMenuOpen {
    menu: String,
    title: Tag,
    items: Vec<Option<ItemStack>>,
    replace: MenuReplace,
    pos: Option<(i32, i32, i32)>,
    extra: Option<Vec<u8>>,
    content: Option<ModMenuData>,
}

struct ModMenuData {
    kind: &'static ModMenuType,
    data: Vec<i32>,
    opening_item: Option<(mod_api::Hand, Option<ItemStack>)>,
    opener_block: Option<Box<str>>,
}

/// Blockless server menus, resolved through the player and current menu handles.
#[allow(dead_code)]
impl Game<'_> {
    pub(super) fn native_conn(&self, uuid: u128) -> Option<u32> {
        let host = self.lan.as_ref()?;
        let g = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && g.participates_online() && g.uuid == uuid)?;
        Some(g.conn)
    }
    /// Opens a chest menu of `rows` rows (1 to 6) titled `title` for the
    /// player, showing `items` (by slot, padded with empty ones), in place of
    /// the menu they had open: its handle, with a new window id each time. The old
    /// menu closes as a close does, its cursor stack going back to the
    /// inventory, and `replace` says whether the client is first sent a close.
    pub(super) fn player_open_chest_menu(
        &mut self,
        player: mod_api::PlayerRef,
        rows: u8,
        title: Text,
        mut items: Vec<Option<ItemStack>>,
        replace: MenuReplace,
    ) -> Result<MenuRef, mod_api::OpenMenuError> {
        use mod_api::OpenMenuError;
        if !self.is_logical_server() {
            return Err(OpenMenuError::PlayerUnavailable);
        }
        let title = component_nbt(&title.component());
        sunrise_protocol::buf::check_network_nbt(&title)
            .map_err(|_| OpenMenuError::TitleNotEncodable)?;
        let guest = self
            .online_player_record(player)
            .filter(|g| !g.dead && !g.disconnect_pending)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let conn = guest.conn;
        if self.menu_click_in_progress(conn) {
            return Err(OpenMenuError::ClickInProgress);
        }
        let menu = format!("minecraft:generic_9x{rows}");
        self.lan
            .as_ref()
            .and_then(|host| host.hub.registry_id("minecraft:menu", &menu))
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        items.resize(usize::from(rows) * 9, None);
        let (window, generation) = self
            .request_native_menu_open(
                conn,
                NativeMenuOpen {
                    menu,
                    title,
                    items,
                    replace,
                    pos: None,
                    extra: None,
                    content: None,
                },
            )
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        Ok(menu_ref(player, window, generation))
    }

    /// Returns the vanilla window id (`1..=100`) and generation when installed. A transaction
    /// or an older publication stages the request with a negative identity and generation zero,
    /// without changing the current menu or allocating a wire id.
    fn request_native_menu_open(&mut self, conn: u32, open: NativeMenuOpen) -> Option<(i32, u64)> {
        if self.menu_open_waits(conn) {
            return Some((self.defer_menu_open(conn, PendingMenuOpen::Native(open)), 0));
        }
        self.in_menu_owner_level(conn, |game| game.install_native_menu(conn, open))?
    }

    pub(super) fn install_pending_menu_open(&mut self, conn: u32, request: PendingMenuOpen) {
        let opened = self.in_menu_owner_level(conn, |game| match request {
            PendingMenuOpen::Native(open) => game.install_native_menu(conn, open).is_some(),
            PendingMenuOpen::ModMenu(open) => {
                matches!(game.finish_open_mod_menu(conn, open), Ok(Some(_)))
            }
            PendingMenuOpen::Block(player, entity) => {
                matches!(game.finish_player_open_menu(conn, player, entity), Ok(true))
            }
            PendingMenuOpen::Ender(player, title) => {
                matches!(game.finish_open_ender_chest(conn, player, title), Ok(true))
            }
        });
        if opened != Some(true) {
            log::warn!("deferred menu open for connection {conn} was refused");
        }
    }

    fn install_native_menu(&mut self, conn: u32, open: NativeMenuOpen) -> Option<(i32, u64)> {
        let guest = self.lan.as_ref()?.hub.guest(conn)?;
        if !guest.placed || !guest.participates_online() {
            return None;
        }
        if guest.disconnect_pending || (open.content.is_some() && guest.dead) {
            return None;
        }
        if self.menu_click_in_progress(conn) {
            return None;
        }
        let NativeMenuOpen {
            menu,
            title,
            items,
            replace,
            pos,
            extra,
            content,
        } = open;
        self.guest_replace_menu(conn, replace == MenuReplace::CloseFirst, |game| {
            let gi = game.guest_index(conn)?;
            // The first contents show the player's own inventory and cursor, in their turn too.
            let opened = game.with_own_record(gi, |lan, _, _| {
                if lan.hub.guests[gi].disconnect_pending {
                    return None;
                }
                let window = lan.hub.open_menu_window_at(
                    conn,
                    &menu,
                    title,
                    pos,
                    Slots::Native,
                    items,
                    extra,
                    content.as_ref().map(|c| c.kind),
                )?;
                if let Some(content) = content {
                    lan.hub.guests.update(gi, |g| {
                        let open = g.menu.as_mut().expect("the window was just installed");
                        open.blockless_data = content.data;
                        open.opening_item = content.opening_item;
                        open.opener_block = content.opener_block;
                        g.send_menu_data_snapshot();
                    });
                }
                Some((window, lan.hub.guests[gi].menu_generation))
            })?;
            game.player_menu_opening(conn);
            Some(opened)
        })
    }
    /// `ServerPlayer.containerCounter`: the window id the player's last opened menu took.
    pub(super) fn player_container_counter(&self, player: mod_api::PlayerRef) -> Option<i32> {
        if !self.is_logical_server() {
            return None;
        }
        Some(self.online_player_record(player)?.next_window)
    }
    /// `ServerPlayer.nextContainerCounter` with no menu after it: the counter moves as an open
    /// moves it, and nothing opens or closes.
    pub(super) fn player_next_container_counter(
        &mut self,
        player: mod_api::PlayerRef,
    ) -> Option<i32> {
        if !self.is_logical_server() {
            return None;
        }
        let conn = self.online_player_record(player)?.conn;
        self.lan
            .as_mut()?
            .hub
            .update_guest(conn, |guest| guest.next_container_counter())
    }
    /// `ServerPlayer.openMenu(provider, extraDataWriter)` for a content mod menu type and no
    /// block: the type's container slots show empty and their clicks reach the mods, as
    /// `player_open_chest_menu`'s do; `extra` and `pos` go first in its `sunrise:menu_extra`
    /// payload, and `replace` is the provider's choice of the client's close, as for
    /// `player_open_chest_menu`.
    pub(super) fn player_open_mod_menu(
        &mut self,
        player: mod_api::PlayerRef,
        request: ModMenuOpen,
    ) -> Result<Option<MenuRef>, mod_api::OpenMenuError> {
        use mod_api::OpenMenuError;
        use sunrise_protocol::menu_extra;

        let ModMenuOpen {
            menu,
            title,
            extra,
            data,
            pos,
            opening_hand,
            replace,
            ..
        } = request;
        let title = title.try_into_engine(&())?;
        let id = mod_api::__private::menu_key_id(menu).ok_or(OpenMenuError::NotModMenu)?;
        let kind =
            sunrise_client::containers::mod_menu_type(&id).ok_or(OpenMenuError::NotModMenu)?;
        if kind.owned_slots.is_some() && kind.crafting.is_some() {
            return Err(OpenMenuError::OwnedSlotsWithCrafting);
        }
        if extra.len() > menu_extra::MAX_DATA {
            return Err(OpenMenuError::TooLarge);
        }
        if data.len() != kind.data_slots {
            return Err(OpenMenuError::DataCountMismatch);
        }
        let title = component_nbt(&title.component());
        sunrise_protocol::buf::check_network_nbt(&title)
            .map_err(|_| OpenMenuError::TitleNotEncodable)?;
        // A live handle of a connected player; a fake player has no connection.
        let guest = self
            .online_player_record(player)
            .filter(|g| !g.dead && !g.disconnect_pending)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let revision = if !data.is_empty() || !kind.player_inventory || kind.slots == 0 {
            menu_extra::LIVE_DATA_REVISION
        } else {
            1 // Client must have negotiated menu_extra revision 1 or higher.
        };
        if !sunrise_client::containers::menu_reaches(&guest.brand, &id)
            || guest.sender.menu_extra_rev < revision
        {
            return Err(OpenMenuError::PlayerUnavailable);
        }
        let conn = guest.conn;
        self.lan
            .as_ref()
            .and_then(|host| host.hub.registry_id("minecraft:menu", &id))
            .ok_or(OpenMenuError::NotModMenu)?;
        // A speculative quick move or merchant action may still own the window.
        if self.menu_click_in_progress(conn) {
            return Err(OpenMenuError::ClickInProgress);
        }
        let open = PendingModMenuOpen {
            player,
            id: id.to_string(),
            kind,
            title,
            extra,
            data,
            pos,
            opening_hand,
            replace,
        };
        if self.menu_open_waits(conn) {
            let staged = self.defer_menu_open(conn, PendingMenuOpen::ModMenu(open));
            return Ok(Some(menu_ref(player, staged, 0)));
        }
        self.in_menu_owner_level(conn, |game| game.finish_open_mod_menu(conn, open))
            .ok_or(OpenMenuError::PlayerUnavailable)?
    }

    fn finish_open_mod_menu(
        &mut self,
        conn: u32,
        open: PendingModMenuOpen,
    ) -> Result<Option<MenuRef>, mod_api::OpenMenuError> {
        use mod_api::OpenMenuError;
        use sunrise_protocol::menu_extra;

        let PendingModMenuOpen {
            player,
            id,
            kind,
            title,
            extra,
            data,
            pos,
            opening_hand,
            replace,
        } = open;
        let guest = self
            .online_player_record(player)
            .filter(|g| g.conn == conn && !g.dead && !g.disconnect_pending)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let revision = if !data.is_empty() || !kind.player_inventory || kind.slots == 0 {
            menu_extra::LIVE_DATA_REVISION
        } else {
            1
        };
        if !sunrise_client::containers::menu_reaches(&guest.brand, &id)
            || guest.sender.menu_extra_rev < revision
        {
            return Err(OpenMenuError::PlayerUnavailable);
        }
        self.lan
            .as_ref()
            .and_then(|host| host.hub.registry_id("minecraft:menu", &id))
            .ok_or(OpenMenuError::NotModMenu)?;
        if self.menu_click_in_progress(conn) {
            return Err(OpenMenuError::ClickInProgress);
        }
        if !self
            .guest_may_open(conn, &id, pos)
            .ok_or(OpenMenuError::PlayerUnavailable)?
        {
            return Ok(None);
        }
        let guest = self
            .online_player_record(player)
            .filter(|g| g.conn == conn && !g.dead && !g.disconnect_pending)
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        let opening_item = opening_hand.map(|hand| (hand, self.held_by(guest, hand).1.cloned()));
        let pos = pos.map(|[x, y, z]| (x, y, z));
        let opener_block = pos
            .and_then(|pos| self.menu_opener_block(pos, &id))
            .map(Into::into);
        let (window, generation) = self
            .install_native_menu(
                conn,
                NativeMenuOpen {
                    menu: id,
                    title,
                    items: vec![None; kind.slots],
                    replace,
                    pos,
                    extra: Some(extra),
                    content: Some(ModMenuData {
                        kind,
                        data,
                        opening_item,
                        opener_block,
                    }),
                },
            )
            .ok_or(OpenMenuError::PlayerUnavailable)?;
        Ok(Some(menu_ref(player, window, generation)))
    }
    /// Internal handle lookup for the native menu `uuid` has open as window `window`, for
    /// server-side recipe-book and slot-backing paths.
    pub(super) fn native_menu_ref(&self, uuid: u128, window: i32) -> Option<MenuRef> {
        let conn = self.native_conn(uuid)?;
        self.lan
            .as_ref()?
            .hub
            .guest(conn)?
            .menu
            .as_ref()
            .filter(|m| m.window == window && m.slots == Slots::Native && !m.removed)?;
        self.player_container_menu(self.player(uuid)?)
            .filter(|menu| menu.fields().window == window)
    }
    /// Replaces what the player's open native menu shows, without reopening it
    /// (`AbstractContainerMenu.sendAllDataToRemote`): the whole contents in one packet.
    pub(super) fn menu_set_items(
        &mut self,
        menu: MenuRef,
        items: Vec<Option<ItemStack>>,
    ) -> Result<(), mod_api::MenuItemsRefusal> {
        use mod_api::MenuItemsRefusal;
        if !self.is_logical_server() {
            return Err(MenuItemsRefusal::Client);
        }
        if self.native_lifecycle.epoch().is_none() {
            return Err(MenuItemsRefusal::Unavailable);
        }
        let player = menu.fields().player;
        let window = menu.fields().window;
        if self.online_player_record(player).is_none() {
            return Err(MenuItemsRefusal::NoPlayer);
        }
        let guest = self
            .native_menu_guest(menu)
            .ok_or(MenuItemsRefusal::NotOpen)?;
        let conn = guest.conn;
        let open = guest
            .menu
            .as_ref()
            .filter(|m| m.slots == Slots::Native && !m.removed)
            .ok_or(MenuItemsRefusal::NotOpen)?;
        if open.owned_slots().is_some() {
            return Err(MenuItemsRefusal::OwnedSlots);
        }
        let size = open.size;
        let gi = self.guest_index(conn).ok_or(MenuItemsRefusal::NoPlayer)?;
        let mut items = items;
        items.resize(size, None);
        let transactional = !self.transactions.is_empty();
        if transactional {
            self.journal_menus(player);
            self.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(gi, |g| g.native_menu = items);
        } else {
            self.with_own_record(gi, |host, _, _| {
                host.hub
                    .guests
                    .update(gi, |g| g.native_menu = items.clone());
                host.hub.send_menu_content(gi, items);
                Some(())
            });
        }
        if transactional {
            self.reply_menu_click(conn, super::lan_menus::ClickReply::Resend(window));
        }
        Ok(())
    }
}

type ContainerPos = (i32, i32, i32);

/// A menu the player opened, including the block entity instances it opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct HeardMenu {
    window: i32,
    generation: u64,
    /// The menu type's name; a mount's inventory has none in vanilla.
    menu: String,
    /// The block the menu is of.
    pos: Option<[i32; 3]>,
    /// Each position and its generation when the menu opened; combined menus hold both halves.
    openers: [Option<(ContainerPos, u64)>; 2],
}

impl GuestMenus {
    pub(super) fn menu_holds(
        &self,
        menu: Option<&Menu>,
        pos: ContainerPos,
        generation: u64,
    ) -> bool {
        self.menu_heard
            .as_ref()
            .zip(menu)
            .is_some_and(|(opened, live)| {
                opened.window == live.window && opened.openers.contains(&Some((pos, generation)))
            })
    }
}

/// What a mount's inventory is called, as vanilla's `HorseInventoryMenu` has
/// no menu type.
pub(super) const MOUNT_MENU: &str = "sunrise:mount_inventory";

/// The menu `conn` has open, whichever way the game keeps it: a block's
/// container, a trade, a crafting table, a workstation, a lectern or a
/// mount's inventory. Not the player's own inventory menu.
pub(super) fn open_menu_of(host: &LanHost, conn: u32) -> Option<(i32, &str, Option<[i32; 3]>)> {
    let g = host.hub.guest(conn)?;
    let play = host.play.get(&conn);
    let menus = host.menus.get(&conn);
    if let Some(m) = &g.menu {
        // A vehicle's menu is keyed by a stand-in, not a block.
        let pos = m
            .pos
            .filter(|p| p.1 != sunrise_client::lan::menu::VEHICLE_KEY_Y)
            .map(|p| [p.0, p.1, p.2]);
        return Some((m.window, &m.registry_id, pos));
    }
    let (window, menu, pos) = if let Some(gm) = menus.and_then(|m| m.merchant.as_ref()) {
        (gm.window, "minecraft:merchant", None)
    } else if let Some((window, _, _, pos)) = menus.and_then(|m| m.crafting.table.as_ref()) {
        (*window, "minecraft:crafting", Some(*pos))
    } else if let Some(st) = menus.and_then(|m| m.station.as_ref()) {
        (
            st.window,
            super::lan_stations::menu_of(st.kind)?.0,
            Some(st.pos),
        )
    } else if let Some(l) = play.and_then(|p| p.lectern.as_ref()) {
        (
            l.window,
            "minecraft:lectern",
            Some([l.key.0, l.key.1, l.key.2]),
        )
    } else {
        let m = play.and_then(|p| p.mount_window.as_ref())?;
        (m.window, MOUNT_MENU, None)
    };
    Some((window, menu, pos))
}

/// Where the game calls the menu hook points.
impl Game<'_> {
    /// `ServerPlayer.openMenu` / `openHorseInventory`: called only after
    /// constructing the menu, queuing its open packet and sending full contents.
    /// Remember it before dispatch so a hook can close it synchronously.
    pub(super) fn player_menu_opening(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some((window, menu, pos)) = open_menu_of(host, conn) else {
            return;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return;
        };
        let openers = guest.menu.as_ref().map_or([None; 2], |m| {
            let positions = match m.slots {
                Slots::Block(at) => [Some(at), None],
                Slots::Double(first, second) => [Some(first), Some(second)],
                Slots::Ender => [m.pos, None],
                Slots::Native => [None; 2],
            };
            positions.map(|pos| {
                pos.and_then(|pos| {
                    self.world
                        .block_entities
                        .generation(&pos)
                        .map(|generation| (pos, generation))
                })
            })
        });
        let menu = HeardMenu {
            window,
            generation: guest.menu_generation,
            menu: menu.to_owned(),
            pos,
            openers,
        };
        host.menus.entry(conn).or_default().menu_heard = Some(menu.clone());
        self.player_menu_changed(conn, menu, true);
    }

    /// `conn`'s tracked menu closes with the guest's state in place
    /// (`ServerPlayer.doCloseContainer`). Close listeners are checked independently
    /// of open listeners.
    pub(in crate::game) fn player_menu_closing(&mut self, conn: u32, window: i32, generation: u64) {
        let heard = self
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|p| {
                p.owned_menu = None;
                p.menu_heard
                    .take_if(|menu| menu.window == window && menu.generation == generation)
            });
        if let Some(old) = heard {
            self.player_menu_changed(conn, old, false);
        }
    }

    fn player_menu_changed(&mut self, conn: u32, menu: HeardMenu, open: bool) {
        let hook = if open {
            mod_api::HookId::PlayerContainerOpen
        } else {
            mod_api::HookId::PlayerContainerClose
        };
        let heard = mod_api::__private::listened(hook);
        let openers = menu
            .openers
            .map(|opener| opener.filter(|&(pos, _)| self.mod_menu_at(pos).is_some()));
        if !heard && openers == [None; 2] {
            return;
        }
        let Some(guest) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
            return;
        };
        let uuid = guest.uuid;
        let name = (heard && open).then(|| guest.name.clone());
        let dimension = self.dim.id();
        self.player_hook(conn, move |g| {
            for (pos, generation) in openers.into_iter().flatten() {
                g.mod_container_player_opener_for(pos, open, Some(generation));
            }
            if heard {
                let player = g.player(uuid);
                let handle = player.and_then(|player| {
                    let closing = menu_ref(player, menu.window, menu.generation);
                    g.native_menu_guest(closing)?;
                    Some(closing)
                });
                if let Some(name) = name {
                    g.native_player_container_open(
                        uuid, &name, dimension, &menu.menu, menu.pos, handle,
                    );
                } else if !open && let Some(player) = player {
                    g.native_player_container_close(player, &menu.menu, menu.pos, handle);
                }
            }
        });
    }

    /// Runs a player hook with the guest's state in place, or as it is when
    /// the guest has no body to swap in.
    fn player_hook(&mut self, conn: u32, f: impl FnOnce(&mut Self)) {
        let mut f = Some(f);
        if self.lan_acting != Some(conn) {
            self.as_guest(conn, |g| {
                if let Some(f) = f.take() {
                    f(g);
                }
            });
        }
        if let Some(f) = f {
            f(self);
        }
    }

    /// Delivers a detached `AbstractContainerMenu.removed` event: ends backed slot places
    /// and invokes the owning `MenuPart::menu_removed` (with `player: None` if the player is
    /// already gone).
    pub(super) fn deliver_menu_removed(&mut self, event: NativeMenuEvent) {
        let NativeMenuEvent::Closed {
            uuid,
            window,
            menu,
            block,
            opening_hand,
            owned_slots,
            ..
        } = event
        else {
            return;
        };
        self.menu_places_closed(uuid, window);
        if mod_api::__private::has_part_hook(mod_api::HookId::MenuRemoved) {
            use super::api_values::TryIntoApi;
            let owned_slots: Vec<_> = owned_slots
                .iter()
                .map(|slot| slot.as_ref().and_then(|stack| stack.try_into_api().ok()))
                .collect();
            let player = self.player(uuid);
            self.native_menu_removed(mod_api::__private::MenuRemovedCall {
                menu: &menu,
                block: block.as_deref(),
                player,
                uuid,
                window,
                opening_hand,
                owned_slots: &owned_slots,
            });
        }
    }

    /// The notices the hub collected, dispatched with the hub in `self.lan` and no guest turn active.
    pub(super) fn run_native_menu_events(&mut self) {
        loop {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.hub.native_menu_events.is_empty() {
                return;
            }
            match host.hub.native_menu_events.remove(0) {
                NativeMenuEvent::ReturnCarried { conn, item } => {
                    if let Some(gi) = self.guest_index(conn) {
                        self.lan_place_back(gi, vec![item]);
                    }
                }
                NativeMenuEvent::ReturnProcessingInputs {
                    conn,
                    items,
                    drop_instead,
                } => {
                    if let Some(gi) = self.guest_index(conn) {
                        let dead = self
                            .lan
                            .as_ref()
                            .is_some_and(|host| host.hub.guests[gi].dead);
                        if drop_instead || dead {
                            self.drop_guest_items(gi, items);
                        } else {
                            self.lan_place_back(gi, items);
                        }
                    }
                }
                event @ NativeMenuEvent::Closed { .. } => {
                    self.deliver_menu_removed(event);
                }
                // A write to a backed menu's storage (#3385): its slots looked at again.
                NativeMenuEvent::PlacesChanged { uuid, window } => {
                    self.menu_places_changed(uuid, window)
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "native_menus/container_counter_tests.rs"]
mod container_counter_tests;

#[cfg(test)]
#[path = "native_menus/owned_slots_tests.rs"]
mod owned_slots_tests;

#[cfg(test)]
#[path = "native_menus/container_loot_open_tests.rs"]
mod container_loot_open_tests;

#[cfg(test)]
mod tests {
    use super::super::api_values::TryIntoApi;
    use super::super::native_entity_refs::tests::with_game;
    use super::super::native_item_places::tests::{guest, open_to_lan};
    use super::*;
    use mod_api::{HostTransaction, ItemPlace, ItemResource, ServerHost, TransactionError};
    use std::sync::{Arc, mpsc};
    use sunrise_client::lan::{Out, PlayerSender};
    use sunrise_protocol::{
        buf::Reader,
        packets::play::{self, Clientbound as C, Serverbound as S},
    };

    /// Guest 1 (UUID 123) on a world open to LAN, and what their client is sent.
    fn watched_guest(game: &mut Game<'_>) -> mpsc::Receiver<Out> {
        open_to_lan(game);
        let (tx, rx) = mpsc::channel();
        let mut player = guest(1);
        player.sender = PlayerSender::new(tx, Arc::default());
        game.lan.as_mut().unwrap().hub.guests.push(player);
        rx
    }

    /// The packets sent since the last call.
    pub(super) fn sent(rx: &mpsc::Receiver<Out>) -> Vec<C> {
        rx.try_iter()
            .flat_map(|out| match out {
                Out::Packet(body) => vec![body],
                Out::Packets(bodies) => bodies,
                _ => Vec::new(),
            })
            .map(|body| {
                let mut reader = Reader::new(&body);
                let id = reader.varint().unwrap();
                C::decode(id, reader.rest()).unwrap()
            })
            .collect()
    }

    fn open(game: &mut Game<'_>, rows: u8, title: &str, replace: MenuReplace) -> MenuRef {
        let player = game.player(123).unwrap();
        let host: &mut dyn ServerHost = game;
        player
            .open_chest_menu(
                host,
                rows,
                mod_api::Text::literal(title),
                Vec::new(),
                replace,
            )
            .unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn acting_menu_storage_extraction_rolls_back() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            with_game(|game| {
                let _rx = watched_guest(game);
                let stack = ItemStack::new("minecraft:diamond", 5);
                let resource = ItemResource::of(&stack.clone().try_into_api().unwrap());
                let mut restored = Vec::new();
                for ender in [true, false] {
                    let host = game.lan.as_mut().unwrap();
                    if ender {
                        host.hub
                            .guests
                            .update(0, |g| g.ender_items[0] = Some(stack.clone()));
                        host.hub.open_container(
                            1,
                            "minecraft:generic_9x3",
                            sunrise_core::nbt::Tag::String("ender".into()),
                            (0, 64, 0),
                            sunrise_client::lan::menu::Slots::Ender,
                            vec![Some(stack.clone())],
                            None,
                        );
                    } else {
                        host.hub
                            .open_native_menu(
                                1,
                                3,
                                sunrise_core::nbt::Tag::String("native".into()),
                                vec![Some(stack.clone())],
                            )
                            .unwrap();
                    }
                    game.as_guest(1, |game| {
                        let menu = game
                            .player_container_menu(game.player(123).unwrap())
                            .unwrap();
                        let place = ItemPlace::MenuSlot { menu, slot: 0 };
                        let mut inner_restored = None;
                        let host: &mut dyn ServerHost = game;
                        let aborted = host.transaction(|host, tx| {
                            assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                            let inner = host.transaction(|host, tx| {
                                assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                                Err::<(), ()>(())
                            });
                            assert_eq!(inner, Err(TransactionError::Aborted(())));
                            inner_restored =
                                place.with_stack(host, |stack| stack.project()).flatten();
                            assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                            Err::<(), ()>(())
                        });
                        assert_eq!(aborted, Err(TransactionError::Aborted(())));
                        restored.push((
                            inner_restored,
                            place.with_stack(host, |stack| stack.project()).flatten(),
                        ));
                        // Player and menu writes share the ender snapshot, not competing copies.
                        let aborted = host.transaction(|host, tx| {
                            assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                            assert_eq!(menu.cursor().insert(host, tx, &resource, 1), Ok(1));
                            Err::<(), ()>(())
                        });
                        assert_eq!(aborted, Err(TransactionError::Aborted(())));
                        assert_eq!(
                            place.with_stack(host, |stack| stack.project()).flatten(),
                            Some(stack.clone().try_into_api().unwrap())
                        );
                        assert_eq!(menu.carried(host), Some(None));
                        // A committed child lends its first snapshot to the parent.
                        let aborted = host.transaction(|host, _| {
                            host.transaction(|host, tx| {
                                assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                                Ok::<_, ()>(())
                            })
                            .unwrap();
                            Err::<(), ()>(())
                        });
                        assert_eq!(aborted, Err(TransactionError::Aborted(())));
                        assert_eq!(
                            place.with_stack(host, |stack| stack.project()).flatten(),
                            Some(stack.clone().try_into_api().unwrap())
                        );
                    });
                    game.guest_close_all(1);
                }
                let expected = |n| {
                    Some(
                        ItemStack::new("minecraft:diamond", n)
                            .try_into_api()
                            .unwrap(),
                    )
                };
                assert_eq!(
                    restored,
                    vec![(expected(4), expected(5)), (expected(4), expected(5))]
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn container_open_handles_follow_supported_menus() {
        use mod_api::{Hooks, ModEntry, Mods, PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        struct Watch(Arc<Mutex<Vec<String>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Watch {
            fn player_container_open(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                match event.menu_type {
                    "minecraft:furnace" => {
                        let menu = event.menu.expect("furnace has a handle during open");
                        assert_eq!(
                            menu.slot(host, 3),
                            Some(Some(
                                ItemStack::new("minecraft:iron_ingot", 5)
                                    .try_into_api()
                                    .unwrap()
                            ))
                        );
                        assert_eq!(
                            menu.slot(host, 0),
                            Some(Some(
                                ItemStack::new("minecraft:iron_ore", 3)
                                    .try_into_api()
                                    .unwrap()
                            ))
                        );
                    }
                    "minecraft:crafting" => assert!(event.menu.is_none()),
                    "minecraft:generic_9x3" => assert!(event.menu.is_some()),
                    other => panic!("unexpected menu {other}"),
                }
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("open {}", event.menu_type));
            }
            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.menu_type));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new("watch", Box::new(Watch(events.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let _rx = watched_guest(game);
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.inventory[9] = Some(ItemStack::new("minecraft:iron_ingot", 5));
                });
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                for name in [
                    "minecraft:chest",
                    "minecraft:furnace",
                    "minecraft:crafting_table",
                ] {
                    let state = game.pack.assets.registry.default_state(name).unwrap();
                    assert_eq!(
                        game.level_set_block(
                            level,
                            [0, 64, 0],
                            state,
                            mod_api::BlockUpdateFlags::NEIGHBORS
                        ),
                        Ok(true)
                    );
                    if name == "minecraft:furnace" {
                        let Some(super::super::BlockEntity::Furnace(furnace)) =
                            game.world.block_entities.get_mut(&(0, 64, 0))
                        else {
                            panic!("furnace block entity")
                        };
                        furnace.items[0] = Some(ItemStack::new("minecraft:iron_ore", 3));
                    }
                    if name == "minecraft:crafting_table" {
                        game.lan_open_crafting(1, [0, 64, 0]);
                    } else {
                        assert_eq!(
                            game.lan_open_container(1, (0, 64, 0)),
                            super::super::lan_actions::MenuOpen::Opened
                        );
                    }
                    // The same accessor works in any acting server turn, not only the notice.
                    game.as_guest(1, |game| {
                        let player = game.player(123).unwrap();
                        let menu = game.player_container_menu(player);
                        if name == "minecraft:crafting_table" {
                            assert!(menu.is_none());
                        } else {
                            let menu = menu.unwrap();
                            assert!(game.menu_slot_count(menu).is_some());
                            // The cursor is swapped out of Guest for this turn.
                            let cursor = Some(ItemStack::new("minecraft:stone", 2));
                            game.carried = cursor.clone();
                            assert_eq!(game.menu_carried(menu), Some(cursor));
                            game.carried = None;
                            let fields = menu.fields();
                            let stale = mod_api::__private::game::menu_ref(
                                player,
                                fields.window,
                                fields.generation + 1,
                            );
                            assert_eq!(game.menu_slot(stale, 0), None);
                            let wrong_window = mod_api::__private::game::menu_ref(
                                player,
                                fields.window + 1,
                                fields.generation,
                            );
                            assert_eq!(game.menu_slot(wrong_window, 0), None);
                        }
                    });
                    game.guest_close_all(1);
                }
                assert_eq!(
                    *events.lock().unwrap(),
                    [
                        "open minecraft:generic_9x3",
                        "close minecraft:generic_9x3",
                        "open minecraft:furnace",
                        "close minecraft:furnace",
                        "open minecraft:crafting",
                        "close minecraft:crafting",
                    ]
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn container_hooks_follow_actual_open_boundaries_without_a_tick() {
        use mod_api::{Hooks, ModEntry, Mods, PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menus::tests::container_hooks_follow_actual_open_boundaries_without_a_tick",
            "SUNRISE_CONTAINER_BOUNDARY_CASE",
            &["same_tick", "replace", "mount", "guest"],
        ) {
            return;
        }
        struct Watch {
            events: Arc<Mutex<Vec<(bool, String)>>>,
            packets: Arc<Mutex<Option<mpsc::Receiver<Out>>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Watch {
            fn player_container_open(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                // Both packets must precede the callback, not just the open's return.
                let packets = self.packets.lock().unwrap();
                let packets = sent(packets.as_ref().unwrap());
                let open = packets
                    .iter()
                    .find_map(|packet| match packet {
                        C::OpenScreen(p) => Some(p.container),
                        C::MountScreenOpen(p) => Some(p.container),
                        _ => None,
                    })
                    .unwrap();
                assert!(packets.iter().any(|packet| {
                    matches!(packet, C::ContainerSetContent(p) if p.container == open)
                }));
                self.events
                    .lock()
                    .unwrap()
                    .push((true, event.menu_type.to_string()));
            }
            fn player_container_close(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.events
                    .lock()
                    .unwrap()
                    .push((false, event.menu_type.to_string()));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let packets = Arc::new(Mutex::new(None));
        let mods: Mods = vec![ModEntry::new(
            "watch",
            Box::new(Watch {
                events: events.clone(),
                packets: packets.clone(),
            }),
        )]
        .into();
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            *packets.lock().unwrap() = Some(watched_guest(game));
            let case = std::env::var("SUNRISE_CONTAINER_BOUNDARY_CASE").unwrap();
            // Own inventory and unsuccessful construction have no open boundary.
            game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 0 }));
            game.lan_open_mount(1, u32::MAX);
            assert_eq!(
                game.lan_open_container(1, (0, 64, 0)),
                super::super::lan_actions::MenuOpen::NoProvider
            );
            assert!(events.lock().unwrap().is_empty());
            if case == "mount" {
                game.mobs.push(sunrise_client::mobs::Mob::new(
                    31,
                    game.res.mob_types().get("minecraft:horse").unwrap().clone(),
                    sunrise_core::geometry::Vec3::new(0.0, 0.0, 0.0),
                    0.0,
                ));
                game.lan_open_mount(1, 31);
                assert_eq!(
                    open_menu_of(game.lan.as_ref().unwrap(), 1),
                    Some((1, "sunrise:mount_inventory", None))
                );
                game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "sunrise:mount_inventory".into()),
                        (false, "sunrise:mount_inventory".into()),
                    ]
                );
                return;
            }
            let first = if case == "guest" {
                let mut menu = None;
                game.as_guest(1, |game| {
                    menu = Some(open(game, 1, "first", MenuReplace::CloseFirst));
                });
                menu.unwrap()
            } else {
                open(game, 1, "first", MenuReplace::CloseFirst)
            };
            let first_window = first.container_id(game).unwrap();
            assert_eq!(first_window, 1);
            assert_eq!(
                open_menu_of(game.lan.as_ref().unwrap(), 1),
                Some((1, "minecraft:generic_9x1", None))
            );
            assert_eq!(
                *events.lock().unwrap(),
                vec![(true, "minecraft:generic_9x1".into())]
            );
            if case == "replace" {
                let second = open(game, 2, "second", MenuReplace::InPlace);
                assert_eq!(second.container_id(game), Some(2));
                // A stale close must not notify the replacement.
                game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                        (true, "minecraft:generic_9x2".into()),
                    ]
                );
                assert!(second.close_container(game));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                        (true, "minecraft:generic_9x2".into()),
                        (false, "minecraft:generic_9x2".into()),
                    ]
                );
            } else {
                game.lan_guest_packet(
                    1,
                    S::ContainerClose(play::ContainerClose {
                        container: first_window,
                    }),
                );
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                    ]
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_slot_written_outside_a_click_is_one_set_slot_at_the_next_sync() {
        with_game(|game| {
            let rx = watched_guest(game);
            let menu = open(game, 1, "shop", MenuReplace::CloseFirst);
            let player = game.player(123).unwrap();
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), Some(1));
            assert_eq!(player.container_menu(host), Some(menu));
            sent(&rx);

            let diamond = ItemStack::new("minecraft:diamond", 1)
                .try_into_api()
                .unwrap();
            let slot = ItemPlace::MenuSlot { menu, slot: 4 };
            let written =
                host.transaction(|host, tx| slot.insert(host, tx, &ItemResource::of(&diamond), 3));
            assert_eq!(written, Ok(3));
            assert!(sent(&rx).is_empty(), "nothing before the tick's sync");

            game.lan.as_mut().unwrap().hub.sync_containers(&game.world);
            let packets = sent(&rx);
            assert_eq!(packets.len(), 1, "{packets:?}");
            let C::ContainerSetSlot(set) = &packets[0] else {
                panic!("{packets:?}");
            };
            // The open's content took state 1; this is the next.
            assert_eq!((set.container, set.state, set.slot), (1, 2, 4));
            assert_eq!(set.item.as_ref().map(|item| item.count), Some(3));
            game.lan.as_mut().unwrap().hub.sync_containers(&game.world);
            assert!(sent(&rx).is_empty(), "sent once");

            // Closed, the menu answers nothing.
            assert!(menu.close_container(game));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), None);
            assert_ne!(player.container_menu(host), Some(menu));
        });
    }

    /// `MenuRef::set_items` sends the whole contents of the open menu in one packet for its
    /// window, padded to its slots and cut at them, and the screen, the title and the cursor stay.
    #[test]
    #[ignore = "daily"]
    fn a_menus_items_are_replaced_in_one_packet_without_reopening() {
        with_game(|game| {
            let rx = watched_guest(game);
            let menu = open(game, 1, "shop", MenuReplace::CloseFirst);
            let window = menu.container_id(game).unwrap();
            let stone = ItemStack::new("minecraft:stone", 3);
            let cursor = ItemStack::new("minecraft:dirt", 2);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(cursor.clone()));
            sent(&rx);
            let api = |stack: &ItemStack| stack.clone().try_into_api().unwrap();
            let host: &mut dyn ServerHost = game;
            // Two stacks, then enough empty slots to be cut at the menu's nine.
            let mut items = vec![None, Some(api(&stone))];
            items.resize(12, None);
            items[11] = Some(api(&stone));
            assert_eq!(menu.set_items(host, items), Ok(()));
            let guest = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.native_menu.len(), 9, "cut at the menu's slots");
            assert_eq!(guest.native_menu[1], Some(stone.clone()));
            assert_eq!(
                guest.native_menu[8], None,
                "the tenth to twelfth are dropped"
            );
            assert_eq!(guest.carried, Some(cursor), "the cursor stays");
            let packets = sent(&rx);
            assert_eq!(packets.len(), 1, "one packet: {packets:?}");
            let C::ContainerSetContent(content) = &packets[0] else {
                panic!("{packets:?}");
            };
            assert_eq!(content.container, window);
            assert_eq!(content.items.len(), 9 + 36, "the menu, then the inventory");
            assert_eq!(content.items[1].as_ref().map(|item| item.count), Some(3));
            assert!(
                content.items[0].is_none(),
                "padded and empty slots are empty"
            );
            assert!(content.carried.is_some(), "the cursor is sent with it");
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), Some(window), "still the open menu");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn bulk_menu_updates_send_the_live_owner_and_abort_without_packets() {
        with_game(|game| {
            let rx = watched_guest(game);
            let menu = open(game, 1, "display", MenuReplace::CloseFirst);
            let window = menu.container_id(game).unwrap();
            let mut outer = guest(2);
            outer.inventory[3] = Some(ItemStack::new("minecraft:diamond", 7));
            outer.carried = Some(ItemStack::new("minecraft:dirt", 9));
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.push(outer);
            hub.guests.update(0, |g| {
                g.inventory[5] = Some(ItemStack::new("minecraft:gold_ingot", 2));
                g.carried = Some(ItemStack::new("minecraft:stone", 4));
            });
            sent(&rx);
            let items = |count| {
                vec![Some(
                    ItemStack::new("minecraft:stick", count)
                        .try_into_api()
                        .unwrap(),
                )]
            };
            let check = |count| {
                let packets = sent(&rx);
                let contents: Vec<_> = packets
                    .iter()
                    .filter_map(|p| match p {
                        C::ContainerSetContent(content) => Some(content),
                        _ => None,
                    })
                    .collect();
                let [content] = contents.as_slice() else {
                    panic!("{packets:?}");
                };
                assert_eq!(content.container, window);
                assert_eq!(content.items[0].as_ref().map(|s| s.count), Some(count));
                assert_eq!(content.items[9 + 27 + 5].as_ref().map(|s| s.count), Some(2));
                assert!(content.items[9 + 27 + 3].is_none());
                assert_eq!(content.carried.as_ref().map(|s| s.count), Some(4));
            };
            game.as_guest(2, |game| {
                game.as_guest(1, |game| {
                    assert_eq!(menu.set_items(game, items(1)), Ok(()));
                    check(1);
                    let aborted: Result<(), mod_api::TransactionError<()>> =
                        game.transaction(|game, _| {
                            assert_eq!(menu.set_items(game, items(3)), Ok(()));
                            Err(())
                        });
                    assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
                    assert!(sent(&rx).is_empty());
                    assert_eq!(
                        menu.slot(game, 0).flatten().map(|stack| stack.count()),
                        Some(1)
                    );
                    let committed: Result<(), mod_api::TransactionError<()>> =
                        game.transaction(|game, _| {
                            assert_eq!(menu.set_items(game, items(5)), Ok(()));
                            assert!(sent(&rx).is_empty());
                            Ok(())
                        });
                    assert_eq!(committed, Ok(()));
                    check(5);
                })
                .unwrap();
                assert_eq!(game.carried.as_ref().unwrap().count, 9);
            })
            .unwrap();
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0]
                    .carried
                    .as_ref()
                    .unwrap()
                    .count,
                4
            );
        });
    }

    /// A menu opened in the player's own turn, nested in another player's, clears and shows
    /// that player's own cursor and inventory: none of the outer player's stacks reach them.
    #[test]
    #[ignore = "daily"]
    fn a_menu_opened_in_a_nested_turn_sends_the_openers_own_inventory_and_cursor() {
        with_game(|game| {
            let rx = watched_guest(game);
            open(game, 1, "old", MenuReplace::CloseFirst);
            let mut outer = guest(2);
            outer.inventory[3] = Some(ItemStack::new("minecraft:diamond", 7));
            outer.carried = Some(ItemStack::new("minecraft:dirt", 9));
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.push(outer);
            hub.guests.update(0, |g| {
                g.inventory[5] = Some(ItemStack::new("minecraft:gold_ingot", 2));
                g.carried = Some(ItemStack::new("minecraft:stone", 4));
            });
            sent(&rx);
            let menu = game
                .as_guest(2, |game| {
                    game.as_guest(1, |game| open(game, 1, "pack", MenuReplace::CloseFirst))
                })
                .flatten()
                .unwrap();
            let window = menu.container_id(game).unwrap();
            let contents: Vec<_> = sent(&rx)
                .into_iter()
                .filter_map(|packet| match packet {
                    C::ContainerSetContent(content) => Some(content),
                    _ => None,
                })
                .collect();
            // The previous cursor's stone is back in the inventory of the new menu.
            let windows: Vec<_> = contents.iter().map(|c| c.container).collect();
            assert_eq!(windows, [window]);
            for content in &contents {
                let mut counts: Vec<_> = content.items.iter().flatten().map(|s| s.count).collect();
                counts.sort_unstable();
                assert_eq!(
                    counts,
                    [2, 4],
                    "window {}: their gold and stone",
                    content.container
                );
                assert!(content.carried.is_none(), "window {}", content.container);
            }
            let hub = &game.lan.as_ref().unwrap().hub;
            assert_eq!(hub.guests[0].carried, None);
            assert_eq!(
                hub.guests[1].carried,
                Some(ItemStack::new("minecraft:dirt", 9)),
                "the outer player's cursor stays theirs"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_entity_menu_parks_only_the_active_guests_cursor() {
        use mod_api::BlockUpdateFlags;
        use sunrise_client::chunk::Chunk;

        with_game(|game| {
            let rx = watched_guest(game);
            let opener_cursor = ItemStack::new("minecraft:stone", 4);
            let outer_cursor = ItemStack::new("minecraft:diamond", 7);
            let opener_slot = ItemStack::new("minecraft:gold_ingot", 2);
            let mut outer = guest(2);
            outer.carried = Some(outer_cursor.clone());
            outer.inventory[0] = Some(ItemStack::new("minecraft:dirt", 9));
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.push(outer);
            hub.guests.update(0, |g| {
                g.carried = Some(opener_cursor.clone());
                g.inventory[0] = Some(opener_slot.clone());
            });
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let entity = game.block_entity(level, [0, 64, 0]).unwrap();
            let opener = game.player(123).unwrap();
            sent(&rx);
            game.as_guest(2, |game| {
                game.as_guest(1, |game| {
                    let host: &mut dyn ServerHost = game;
                    assert_eq!(opener.open_menu(host, entity), Ok(true));
                    assert_eq!(
                        game.carried, None,
                        "the new menu starts with an empty cursor"
                    );
                })
                .unwrap();
                assert_eq!(game.carried, Some(outer_cursor.clone()));
            })
            .unwrap();
            let hub = &game.lan.as_ref().unwrap().hub;
            let window = hub.guests[0].menu.as_ref().unwrap().window;
            let contents = sent(&rx)
                .into_iter()
                .find_map(|p| match p {
                    C::ContainerSetContent(content) if content.container == window => Some(content),
                    _ => None,
                })
                .unwrap();
            assert!(contents.carried.is_none());
            let shown: Vec<_> = contents
                .items
                .iter()
                .flatten()
                .map(|item| hub.stack_from_network(item))
                .collect();
            assert_eq!(shown, [opener_slot]);
            game.lan_guest_packet(
                1,
                S::ContainerClose(play::ContainerClose { container: window }),
            );
            let hub = &game.lan.as_ref().unwrap().hub;
            assert_eq!(hub.guests[0].carried, Some(opener_cursor));
            assert_eq!(hub.guests[1].carried, Some(outer_cursor));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn shutdown_saves_current_and_parked_inventory_cursors() {
        with_game(|game| {
            let world = tempfile::tempdir().unwrap();
            game.world_root = world.path().to_owned();
            let _rx = watched_guest(game);
            let current = ItemStack::new("minecraft:lapis_lazuli", 3);
            let parked = ItemStack::new("minecraft:iron_ingot", 4);
            let open_cursor = ItemStack::new("minecraft:quartz", 5);
            let pending = ItemStack::new("minecraft:gold_ingot", 6);
            let mut other = guest(2);
            other.carried = Some(current.clone());
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.push(other);
            hub.guests.update(0, |g| g.carried = Some(parked.clone()));
            open(game, 1, "open", MenuReplace::CloseFirst);
            let hub = &mut game.lan.as_mut().unwrap().hub;
            assert!(hub.guests[0].inventory.iter().all(Option::is_none));
            hub.guests
                .update(0, |g| g.carried = Some(open_cursor.clone()));
            hub.guests.push(guest(3));
            hub.open_native_menu(
                3,
                1,
                component_nbt(&Text::literal("retired").component()),
                Vec::new(),
            )
            .unwrap();
            hub.guests.update(2, |g| g.carried = Some(pending.clone()));
            hub.close_container(3); // Shutdown wins the race with the queued return.

            game.save_leaving_guests();
            for (conn, expected) in [
                (1, vec![open_cursor, parked]),
                (2, vec![current]),
                (3, vec![pending]),
            ] {
                let mut reloaded = guest(conn);
                assert!(sunrise_client::lan::playerdata::load(
                    &mut reloaded,
                    world.path()
                ));
                assert_eq!(
                    reloaded.inventory.into_iter().flatten().collect::<Vec<_>>(),
                    expected
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn cursor_overflow_listeners_see_and_write_only_the_openers_inventory() {
        use mod_api::{
            EntityJoinLevelEvent, Flow, PlayerContainerCloseEvent, PlayerContainerOpenEvent,
        };
        use std::sync::Mutex;

        struct Listen(Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listen {
            fn entity_join_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind == "minecraft:item" {
                    let opener = host.player(123).unwrap();
                    let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
                    opener.with_inventory(host, |view| {
                        let mut slots = view.iter();
                        for expected in 0..36 {
                            let (slot, stack) = slots.next().unwrap();
                            assert_eq!((slot, stack.item(), stack.count()), (expected, stone, 64),
                                "the listener sees the opener's full inventory after cursor reinsertion");
                        }
                        assert!(slots.next().is_none());
                    }).unwrap();
                    opener.set_inventory_slot(host, 3, None).unwrap();
                    self.0.lock().unwrap().push("drop");
                }
                Flow::Continue
            }

            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                _: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.0.lock().unwrap().push("close");
            }

            fn player_container_open(
                &self,
                _: &mut dyn ServerHost,
                _: &mut PlayerContainerOpenEvent<'_>,
            ) {
                self.0.lock().unwrap().push("open");
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "overflow-listener",
            Box::new(Listen(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                let _rx = watched_guest(game);
                let mut outer = guest(2);
                outer.inventory[3] = Some(ItemStack::new("minecraft:diamond", 7));
                let outer_inventory = outer.inventory.clone();
                game.lan.as_mut().unwrap().hub.guests.push(outer);
                open(game, 1, "old", MenuReplace::CloseFirst);
                heard.lock().unwrap().clear();
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.inventory
                        .fill(Some(ItemStack::new("minecraft:stone", 64)));
                    g.inventory[0] = Some(ItemStack::new("minecraft:stone", 62));
                    g.carried = Some(ItemStack::new("minecraft:stone", 4));
                });
                let dropped = game.dropped.len();
                game.as_guest(2, |game| {
                    game.as_guest(1, |game| open(game, 1, "new", MenuReplace::CloseFirst))
                        .unwrap();
                })
                .unwrap();
                assert_eq!(*heard.lock().unwrap(), ["drop", "close", "open"]);
                assert_eq!(game.dropped.len(), dropped + 1);
                assert_eq!(
                    game.dropped[dropped].stack,
                    ItemStack::new("minecraft:stone", 2)
                );
                let guests = &game.lan.as_ref().unwrap().hub.guests;
                assert_eq!(
                    guests[0].inventory[3], None,
                    "the listener's write belongs to the opener"
                );
                assert_eq!(guests[1].inventory, outer_inventory);
            });
        });
    }

    /// Each refusal has its own name and sends and changes nothing: the inventory menu or a stale
    /// handle, an offline player, a stack the game refuses, and a client.
    #[test]
    #[ignore = "daily"]
    fn each_menu_items_refusal_has_its_own_name_and_sends_nothing() {
        use mod_api::{MenuItemsRefusal, StackWriteError};
        with_game(|game| {
            let rx = watched_guest(game);
            let player = game.player(123).unwrap();
            let api = |stack: ItemStack| stack.try_into_api().unwrap();
            let one = vec![Some(api(ItemStack::new("minecraft:stone", 1)))];
            let inventory = player.container_menu(game).unwrap();
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                inventory.set_items(host, one.clone()),
                Err(MenuItemsRefusal::NotOpen),
                "inventory menu"
            );
            let stale = open(game, 1, "old", MenuReplace::CloseFirst);
            let menu = open(game, 1, "shop", MenuReplace::CloseFirst);
            game.lan.as_mut().unwrap().hub.guests.push(guest(2));
            let other = game.player(124).unwrap();
            let offline = other
                .open_chest_menu(
                    game,
                    1,
                    mod_api::Text::literal("other"),
                    Vec::new(),
                    MenuReplace::CloseFirst,
                )
                .unwrap();
            game.lan.as_mut().unwrap().hub.guests.remove(1);
            sent(&rx);
            let before = game.lan.as_ref().unwrap().hub.guests[0].native_menu.clone();
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                stale.set_items(host, one.clone()),
                Err(MenuItemsRefusal::NotOpen),
                "stale menu"
            );
            assert_eq!(
                offline.set_items(host, one.clone()),
                Err(MenuItemsRefusal::NoPlayer)
            );
            let mut too_many = api(ItemStack::new("minecraft:stone", 1));
            too_many.set_count(65).unwrap();
            assert_eq!(
                menu.set_items(host, vec![Some(too_many)]),
                Err(MenuItemsRefusal::Stack(StackWriteError::CountExceeded {
                    maximum: 64
                }))
            );
            assert!(sent(&rx).is_empty(), "nothing was sent");
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].native_menu,
                before,
                "nothing was changed"
            );
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "MenuItemsRefusalTest").unwrap();
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.set_items(host, one), Err(MenuItemsRefusal::Client));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_menu_opened_in_place_sends_no_close_returns_the_cursor_and_ignores_the_old_window() {
        with_game(|game| {
            let rx = watched_guest(game);
            let closes = |packets: &[C]| -> Vec<i32> {
                packets
                    .iter()
                    .filter_map(|p| match p {
                        C::ContainerClose(close) => Some(close.container),
                        _ => None,
                    })
                    .collect()
            };
            let opens = |packets: &[C]| -> Vec<(i32, sunrise_core::nbt::Tag)> {
                packets
                    .iter()
                    .filter_map(|p| match p {
                        C::OpenScreen(open) => Some((open.container, open.title.clone())),
                        _ => None,
                    })
                    .collect()
            };
            // From the inventory nothing is open, so nothing closes.
            let first = open(game, 1, "one", MenuReplace::CloseFirst);
            assert_eq!(first.container_id(game), Some(1));
            let packets = sent(&rx);
            assert_eq!(closes(&packets), Vec::<i32>::new());
            assert_eq!(opens(&packets).len(), 1);

            let stone = ItemStack::new("minecraft:stone", 4);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(stone.clone()));
            game.lan.as_mut().unwrap().hub.native_menu_events.clear();
            let second = open(game, 2, "two", MenuReplace::InPlace);
            assert_eq!(second.container_id(game), Some(2));
            let packets = sent(&rx);
            assert_eq!(
                closes(&packets),
                Vec::<i32>::new(),
                "no close: the mouse stays"
            );
            assert_eq!(
                opens(&packets),
                vec![(2, component_nbt(&Text::literal("two").component()))]
            );
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.carried, None, "the cursor stack went back");
            assert_eq!(
                g.inventory.iter().flatten().cloned().collect::<Vec<_>>(),
                vec![stone]
            );
            assert_eq!(g.menu.as_ref().map(|m| (m.window, m.size)), Some((2, 18)));
            assert!(
                game.lan.as_ref().unwrap().hub.native_menu_events.is_empty(),
                "the old menu's teardown already ran synchronously"
            );

            // The old window's click and close land on nothing.
            game.lan.as_mut().unwrap().hub.native_menu_events.clear();
            game.lan_guest_packet(
                1,
                S::ContainerClick(play::ContainerClick {
                    container: 1,
                    state: 1,
                    slot: 0,
                    button: 0,
                    input: 0,
                    changed: Vec::new(),
                    carried: None,
                }),
            );
            game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.native_menu_events,
                Vec::new()
            );
            let host: &mut dyn ServerHost = game;
            assert_eq!(first.container_id(host), None);
            assert_eq!(
                second.container_id(host),
                Some(2),
                "the new menu stays open"
            );

            // The default sends the close first.
            sent(&rx);
            let third = open(game, 3, "three", MenuReplace::CloseFirst);
            assert_eq!(third.container_id(game), Some(3));
            let packets = sent(&rx);
            assert_eq!(closes(&packets), vec![2]);
            assert_eq!(opens(&packets).len(), 1);
            let close_at = packets
                .iter()
                .position(|p| matches!(p, C::ContainerClose(_)))
                .unwrap();
            let open_at = packets
                .iter()
                .position(|p| matches!(p, C::OpenScreen(_)))
                .unwrap();
            assert!(close_at < open_at);
        });
    }

    /// The content menu type shared by the mod-menu fixtures.
    pub(super) const MOD_MENU: &str = "listen:status";

    /// [`with_game`] with [`MOD_MENU`] as the thread's menu types, so a test neither depends on
    /// registering first nor touches the one-shot process table.
    pub(super) fn with_mod_menu_game(test: impl FnOnce(&mut Game<'_>)) {
        sunrise_client::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "status_package".into(),
                9,
                0,
                Vec::new(),
                Vec::new(),
                None,
                None,
                true,
                None,
            )],
            || with_game(test),
        );
    }

    /// [`watched_guest`] on a Sunrise client that reads a menu opened with no block, and the
    /// key of [`MOD_MENU`].
    pub(super) fn mod_menu_guest(game: &mut Game<'_>) -> (mpsc::Receiver<Out>, mod_api::MenuKey) {
        let rx = watched_guest(game);
        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.brand = "sunrise".into();
            g.sender.menu_extra_rev = 1;
        });
        let key = super::super::native_guest_turn_tests::register_test_menu(game, MOD_MENU);
        rx.try_iter().for_each(drop); // Registry setup sent reload packets, not part of these opens.
        (rx, key)
    }

    #[test]
    #[ignore = "daily"]
    fn a_mod_menu_opened_in_place_sends_no_close_and_still_closes_the_old_menu() {
        use mod_api::{PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        const CASE: &str = "SUNRISE_MOD_MENU_REPLACE_CHILD";
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menus::tests::a_mod_menu_opened_in_place_sends_no_close_and_still_closes_the_old_menu",
            CASE,
            &["close_first", "in_place"],
        ) {
            return;
        }
        // What the mods heard, in order.
        static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
        struct Listen;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listen {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_menu_screen(
                    MOD_MENU,
                    Arc::new(
                        |_: &dyn mod_api::ClientHost, _: mod_api::MenuScreenOpen<'_>| {
                            HEARD.lock().unwrap().push("layout".to_owned());
                            None
                        },
                    ),
                )
                .unwrap();
            }
            fn player_container_open(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                HEARD
                    .lock()
                    .unwrap()
                    .push(format!("open {}", event.menu_type));
            }
            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                HEARD
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.menu_type));
            }
        }
        let heard = || std::mem::take(&mut *HEARD.lock().unwrap());
        let case = std::env::var(CASE).unwrap();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("listen", Box::new(Listen))].into();
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        let replace = if case == "close_first" {
            MenuReplace::CloseFirst
        } else {
            MenuReplace::InPlace
        };
        with_mod_menu_game(|game| {
            let (rx, key) = mod_menu_guest(game);
            // Where the game keeps what the mods last heard the player open.
            game.lan.as_mut().unwrap().play.entry(1).or_default();
            let player = game.player(123).unwrap();
            let old = open(game, 3, "old", MenuReplace::CloseFirst);
            assert_eq!(old.container_id(game), Some(1));
            assert_eq!(heard(), ["open minecraft:generic_9x3"]);
            sent(&rx);
            let stone = ItemStack::new("minecraft:stone", 4);
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(stone.clone()));

            let menu = game
                .player_open_mod_menu(player, {
                    let mut request = ModMenuOpen::new(key, mod_api::Text::literal("new"));
                    request.extra = vec![7];
                    request.replace = replace;
                    request
                })
                .unwrap();
            assert_eq!(menu.unwrap().container_id(game), Some(2));
            // The old menu's close, heard once before the new one opens, with
            // no layout callback on the server.
            assert_eq!(
                heard(),
                [
                    "close minecraft:generic_9x3".to_owned(),
                    format!("open {MOD_MENU}")
                ]
            );
            let sent_packets = sent(&rx);
            let packets: Vec<String> = sent_packets
                .iter()
                .map(|p| match p {
                    C::ContainerClose(close) => format!("close {}", close.container),
                    C::CustomPayload(payload) => format!("payload {}", payload.channel),
                    C::OpenScreen(open) => format!("open {}", open.container),
                    C::ContainerSetContent(content) => format!(
                        "content {} state {} slots {}",
                        content.container,
                        content.state,
                        content.items.len()
                    ),
                    other => format!("{other:?}"),
                })
                .collect();
            // Only CloseFirst sends the old window's close. The returned cursor
            // is in the player slots of the new window's first contents.
            let mut expected = vec![
                "payload sunrise:menu_extra",
                "open 2",
                "content 2 state 1 slots 45",
            ];
            if replace == MenuReplace::CloseFirst {
                expected.insert(0, "close 1");
            }
            assert_eq!(packets, expected);
            let Some(C::ContainerSetContent(content)) = sent_packets.last() else {
                panic!("expected the new window's contents");
            };
            let hub = &game.lan.as_ref().unwrap().hub;
            let returned: Vec<_> = content.items[9..]
                .iter()
                .flatten()
                .map(|item| hub.stack_from_network(item))
                .collect();
            assert_eq!(returned.as_slice(), std::slice::from_ref(&stone));
            let g = &hub.guests[0];
            assert_eq!(g.carried, None, "the cursor stack went back");
            assert_eq!(
                g.inventory.iter().flatten().cloned().collect::<Vec<_>>(),
                vec![stone]
            );
            assert_eq!(g.menu.as_ref().map(|m| (m.window, m.size)), Some((2, 9)));
            assert!(
                game.lan.as_ref().unwrap().hub.native_menu_events.is_empty(),
                "the old menu's teardown already ran synchronously"
            );

            // Both notifications were delivered inline, without a second close.
            assert!(heard().is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn mod_menu_backend_payload_and_refusals() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_menus::tests::mod_menu_backend_payload_and_refusals",
            "SUNRISE_MOD_MENU_BACKEND_CHILD",
            &["backend"],
        ) {
            return;
        }
        with_mod_menu_game(|game| {
            let (rx, key) = mod_menu_guest(game);
            let player = game.player(123).unwrap();
            let old = open(game, 1, "old", MenuReplace::CloseFirst);
            let old_window = old.container_id(game).unwrap();
            sent(&rx);
            let bytes = vec![0, 255, 17];
            let menu = game
                .player_open_mod_menu(player, {
                    let mut request = ModMenuOpen::new(key, mod_api::Text::literal("new"));
                    request.extra = bytes.clone();
                    request
                })
                .unwrap()
                .unwrap();
            let window = menu.container_id(game).unwrap();
            assert_eq!(window, old_window + 1);
            let packets = sent(&rx);
            let payload_at = packets
                .iter()
                .position(|p| {
                    matches!(p,
                        C::CustomPayload(p) if p.channel == sunrise_protocol::menu_extra::CHANNEL
                    )
                })
                .unwrap();
            let open_at = packets
                .iter()
                .position(|p| {
                    matches!(p,
                        C::OpenScreen(p) if p.container == window
                    )
                })
                .unwrap();
            assert!(payload_at < open_at, "extra data must precede its window");
            let C::CustomPayload(payload) = &packets[payload_at] else {
                unreachable!()
            };
            assert_eq!(
                sunrise_protocol::menu_extra::decode(&payload.data).unwrap(),
                sunrise_protocol::menu_extra::Message {
                    window,
                    pos: None,
                    data: bytes
                }
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .map(|m| (m.window, m.pos)),
                Some((window, None))
            );

            for (revision, extra, expected) in [
                (
                    1,
                    vec![0; sunrise_protocol::menu_extra::MAX_DATA + 1],
                    mod_api::OpenMenuError::TooLarge,
                ),
                (0, vec![1], mod_api::OpenMenuError::PlayerUnavailable),
            ] {
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.sender.menu_extra_rev = revision;
                });
                game.lan.as_mut().unwrap().hub.native_menu_events.clear();
                assert_eq!(
                    game.player_open_mod_menu(player, {
                        let mut request = ModMenuOpen::new(key, mod_api::Text::literal("refused"));
                        request.extra = extra;
                        request.replace = MenuReplace::InPlace;
                        request
                    },),
                    Err(expected)
                );
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guests[0]
                        .menu
                        .as_ref()
                        .map(|m| m.window),
                    Some(window),
                    "refusal preserves the current menu"
                );
                assert!(game.lan.as_ref().unwrap().hub.native_menu_events.is_empty());
                assert!(sent(&rx).is_empty(), "refusal sends neither close nor open");
            }
            // Hub-level encode refusal also aborts before closing or emitting packets.
            let conn = game.lan.as_ref().unwrap().hub.guests[0].conn;
            assert_eq!(
                game.lan.as_mut().unwrap().hub.open_menu_window(
                    conn,
                    MOD_MENU,
                    sunrise_core::nbt::Tag::String("oversize".into()),
                    None,
                    Slots::Native,
                    vec![None; 9],
                    Some(vec![0; sunrise_protocol::menu_extra::MAX_DATA + 1]),
                ),
                None
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .map(|m| m.window),
                Some(window),
                "oversize hub refusal preserves the current menu"
            );
            assert!(sent(&rx).is_empty(), "oversize hub refusal emits nothing");
            // Payloads past the old 32 KiB limit (up to MAX_DATA) are accepted intact.
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender.menu_extra_rev = 1;
            });
            let large = vec![7; 33 * 1024];
            let large_menu = game
                .player_open_mod_menu(player, {
                    let mut request = ModMenuOpen::new(key, mod_api::Text::literal("large"));
                    request.extra = large.clone();
                    request.replace = MenuReplace::InPlace;
                    request
                })
                .unwrap()
                .unwrap();
            let large_window = large_menu.container_id(game).unwrap();
            let large_packets = sent(&rx);
            let large_payload = large_packets
                .iter()
                .find_map(|p| match p {
                    C::CustomPayload(p) if p.channel == sunrise_protocol::menu_extra::CHANNEL => {
                        Some(&p.data)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                sunrise_protocol::menu_extra::decode(large_payload).unwrap(),
                sunrise_protocol::menu_extra::Message {
                    window: large_window,
                    pos: None,
                    data: large,
                }
            );

            // Block-backed mod menus also require the negotiated opening-format capability: a
            // Sunrise-branded peer that gave no revision answer (rev 0) is refused before closing
            // its active window or sending packets, while a negotiated peer opens it and a vanilla
            // peer keeps stock menus.
            use super::super::lan_actions::MenuOpen;
            use sunrise_client::containers::{BlockEntity, ModContainer};
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_client::chunk::Chunk::empty(0, 0)),
            );
            let mod_pos = (0, 64, 0);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world
                .set(mod_pos.0, mod_pos.1, mod_pos.2, stone)
                .unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    stone.0,
                    std::sync::Arc::new(ModContainer {
                        slots: 9,
                        menu: Some(MOD_MENU.into()),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );
            game.world
                .block_entities
                .insert(mod_pos, BlockEntity::Chest(vec![None; 9]));
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.brand = "sunrise".into();
                g.sender.menu_extra_rev = 0;
            });
            assert_eq!(game.open_block_menu(conn, mod_pos), MenuOpen::Refused);
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .map(|m| m.window),
                Some(large_window),
                "unnegotiated Sunrise peer keeps its current menu"
            );
            assert!(
                sent(&rx).is_empty(),
                "unnegotiated block-backed mod menu refusal sends neither close nor open"
            );
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender.menu_extra_rev = 1;
            });
            assert_eq!(game.open_block_menu(conn, mod_pos), MenuOpen::Opened);
            sent(&rx);
            // A stock menu remains available to a vanilla peer with no menu_extra negotiation.
            let chest_pos = (1, 64, 0);
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            game.world
                .set(chest_pos.0, chest_pos.1, chest_pos.2, chest)
                .unwrap();
            game.world
                .block_entities
                .insert(chest_pos, BlockEntity::Chest(vec![None; 27]));
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.brand = "vanilla".into();
                g.sender.menu_extra_rev = 0;
            });
            assert_eq!(game.open_block_menu(conn, chest_pos), MenuOpen::Opened);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn refused_swap_resends_the_menu_and_undisplayed_hotbar() {
        for (player_inventory, button, menu_slots) in [(false, 2, 1), (true, 40, 37)] {
            sunrise_client::containers::with_mod_menu_types(
                [(
                    MOD_MENU.into(),
                    "status_package".into(),
                    1,
                    0,
                    vec![],
                    vec![],
                    None,
                    None,
                    player_inventory,
                    None,
                )],
                || {
                    with_game(|game| {
                        let (rx, key) = mod_menu_guest(game);
                        let held = ItemStack::new("minecraft:stone", 5);
                        let stored = ItemStack::new("minecraft:dirt", 2);
                        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                            g.sender.menu_extra_rev =
                                sunrise_protocol::menu_extra::LIVE_DATA_REVISION;
                            if player_inventory {
                                g.offhand = Some(held.clone());
                            } else {
                                g.inventory[2] = Some(held.clone());
                            }
                        });
                        let player = game.player(123).unwrap();
                        let menu = player
                            .open_mod_menu(
                                game,
                                ModMenuOpen::new(key, mod_api::Text::literal("Panel")),
                            )
                            .unwrap()
                            .unwrap();
                        let window = menu.container_id(game).unwrap();
                        let host: &mut dyn ServerHost = game;
                        menu.set_items(host, vec![Some(stored.clone().try_into_api().unwrap())])
                            .unwrap();
                        sent(&rx);

                        // With no click listener the menu keeps its stack, but the client
                        // predicted a swap with the undisplayed hotbar or offhand.
                        let click = S::ContainerClick(play::ContainerClick {
                            container: window,
                            state: 0,
                            slot: 0,
                            button,
                            input: sunrise_client::lan::menu::input::SWAP,
                            changed: vec![],
                            carried: None,
                        });
                        let host = game.lan.take().unwrap();
                        let host = game
                            .lan_process_events(
                                host,
                                [sunrise_client::lan::hub::HostEvent::Packet(
                                    1,
                                    Box::new(click),
                                )],
                                &[],
                                [0.0; 3],
                            )
                            .unwrap();
                        let packets = sent(&rx);
                        let [C::ContainerSetContent(menu), correction] = &packets[..] else {
                            panic!(
                                "swap button {button}: expected menu and backing inventory corrections, got {packets:?}"
                            );
                        };
                        assert_eq!((menu.container, menu.items.len()), (window, menu_slots));
                        assert_eq!(
                            host.hub.stack_from_network(menu.items[0].as_ref().unwrap()),
                            stored
                        );
                        let item = match (player_inventory, correction) {
                            (false, C::ContainerSetContent(inventory)) => {
                                assert_eq!((inventory.container, inventory.items.len()), (0, 46));
                                &inventory.items[38]
                            }
                            (true, C::SetPlayerInventory(offhand)) => {
                                assert_eq!(offhand.slot, 40);
                                &offhand.item
                            }
                            _ => {
                                panic!("swap button {button}: unexpected correction {correction:?}")
                            }
                        };
                        assert_eq!(host.hub.stack_from_network(item.as_ref().unwrap()), held);
                    });
                },
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn blockless_menu_data_is_committed_to_only_its_live_generation() {
        use mod_api::{ContainerOpenedEvent, MenuDataRefusal, OpenMenuError};
        use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
        use sunrise_protocol::menu_extra::{DATA_CHANNEL, DataSlot, LIVE_DATA_REVISION};

        struct Veto {
            cancel: Arc<AtomicBool>,
            disconnect: Arc<AtomicBool>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn container_opened(
                &self,
                host: &mut dyn ServerHost,
                _: &mut ContainerOpenedEvent<'_>,
            ) -> mod_api::Flow {
                if self.disconnect.load(Relaxed) {
                    let player = host.players().acting_player().unwrap();
                    assert_eq!(
                        player.disconnect(host, mod_api::Text::literal("leaving")),
                        Ok(true)
                    );
                }
                if self.cancel.load(Relaxed) {
                    mod_api::Flow::Cancel
                } else {
                    mod_api::Flow::Continue
                }
            }
        }
        let veto = Arc::new(AtomicBool::new(false));
        let disconnect = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "veto",
            Box::new(Veto {
                cancel: veto.clone(),
                disconnect: disconnect.clone(),
            }),
        )]
        .into();
        let updates = |packets: Vec<C>| {
            packets
                .into_iter()
                .filter_map(|p| match p {
                    C::CustomPayload(p) if p.channel == DATA_CHANNEL => {
                        Some(DataSlot::decode(&p.data).unwrap())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        mod_platform::testing::with_mods(mods, || {
            sunrise_client::containers::with_mod_menu_types(
                [(
                    MOD_MENU.into(),
                    "status_package".into(),
                    0,
                    2,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    false,
                    None,
                )],
                || {
                    with_game(|game| {
                        let (rx, key) = mod_menu_guest(game);
                        let old = open(game, 3, "old", MenuReplace::CloseFirst);
                        sent(&rx);
                        let player = game.player(123).unwrap();
                        let show = |host: &mut dyn ServerHost, data| {
                            player.open_mod_menu(host, {
                                let mut request =
                                    ModMenuOpen::new(key, mod_api::Text::literal("status"));
                                request.extra = vec![7];
                                request.data = data;
                                request.replace = MenuReplace::InPlace;
                                request
                            })
                        };
                        assert_eq!(show(game, vec![9]), Err(OpenMenuError::DataCountMismatch));
                        assert_eq!(
                            show(game, vec![-80_000, 0]),
                            Err(OpenMenuError::PlayerUnavailable)
                        );
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |g| g.sender.menu_extra_rev = LIVE_DATA_REVISION);
                        let network_title = mod_api::Text::literal("x".repeat(65_536));
                        assert_eq!(
                            player.open_mod_menu(game, {
                                let mut request = ModMenuOpen::new(key, network_title);
                                request.data = vec![1, 2];
                                request
                            }),
                            Err(OpenMenuError::TitleNotEncodable),
                        );
                        veto.store(true, Relaxed);
                        assert_eq!(show(game, vec![-80_000, 0]), Ok(None));
                        veto.store(false, Relaxed);
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |g| g.disconnect_pending = true);
                        assert_eq!(
                            show(game, vec![-80_000, 0]),
                            Err(OpenMenuError::PlayerUnavailable)
                        );
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |g| g.disconnect_pending = false);
                        game.lan.as_mut().unwrap().hub.guests.push(guest(2));
                        game.as_guest(1, |game| {
                            game.as_guest(2, |game| {
                                assert_eq!(
                                    show(game, vec![-80_000, 0]),
                                    Err(OpenMenuError::PlayerUnavailable)
                                );
                            })
                            .unwrap();
                        })
                        .unwrap();
                        assert_eq!(
                            game.menu_kind(old),
                            Some(Some("minecraft:generic_9x3".into()))
                        );
                        assert_eq!(
                            old.set_data_slot(game, 0, 1),
                            Err(MenuDataRefusal::Unsupported)
                        );
                        assert!(sent(&rx).is_empty());

                        let menu = show(game, vec![-80_000, 0]).unwrap().unwrap();
                        let window = menu.container_id(game).unwrap();
                        assert_eq!(player.container_menu(game), Some(menu));
                        assert_eq!(menu.slot_count(game), Some(0));
                        assert!(menu.slot(game, 0).is_none());
                        let packets = sent(&rx);
                        let open_at = packets
                            .iter()
                            .position(|p| matches!(p, C::OpenScreen(p) if p.container == window))
                            .unwrap();
                        assert!(packets.iter().any(|p| matches!(p, C::ContainerSetContent(p) if p.container == window && p.items.is_empty())));
                        assert!(!packets[..open_at].iter().any(
                            |p| matches!(p, C::CustomPayload(p) if p.channel == DATA_CHANNEL)
                        ));
                        assert_eq!(
                            updates(packets),
                            [
                                DataSlot {
                                    window,
                                    index: 0,
                                    value: -80_000
                                },
                                DataSlot {
                                    window,
                                    index: 1,
                                    value: 0
                                },
                            ]
                        );
                        {
                            // Any server caller with the live MenuRef can update a declared
                            // data slot, not only the mod that registered the menu type.
                            let _guard = mod_api::content::crash::enter_native_mod(
                                mod_api::content::crash::ModCall {
                                    id: "listen",
                                    hook: "test",
                                    part: None,
                                },
                            );
                            assert_eq!(menu.set_data_slot(game, 0, 12), Ok(true));
                            assert_eq!(
                                updates(sent(&rx)),
                                [DataSlot {
                                    window,
                                    index: 0,
                                    value: 12,
                                }]
                            );
                            assert_eq!(menu.set_data_slot(game, 0, 12), Ok(false));
                            assert!(sent(&rx).is_empty());
                        }
                        // Updating without an active native mod caller succeeds as well.
                        assert_eq!(menu.set_data_slot(game, 0, -80_000), Ok(true));
                        assert_eq!(
                            updates(sent(&rx)),
                            [DataSlot {
                                window,
                                index: 0,
                                value: -80_000,
                            }]
                        );
                        // A full resend of this window has no hotbar slots, so a pending
                        // inventory update must remain pending.
                        let hub = &mut game.lan.as_mut().unwrap().hub;
                        hub.guests.update(0, |g| {
                            assert_eq!(g.menu_sent[36], None);
                            g.inventory[0] = Some(ItemStack::new("minecraft:stone", 1));
                        });
                        hub.resend_menu(1, window, &game.world);
                        assert_eq!(hub.guests[0].menu_sent[36], None);
                        let packets = sent(&rx);
                        assert!(matches!(packets.first(), Some(C::ContainerSetContent(p))
                            if p.container == window && p.items.is_empty()
                        ));
                        let snapshot = [
                            DataSlot {
                                window,
                                index: 0,
                                value: -80_000,
                            },
                            DataSlot {
                                window,
                                index: 1,
                                value: 0,
                            },
                        ];
                        assert_eq!(updates(packets), snapshot);
                        assert_eq!(
                            menu.set_data_slot(game, 2, 1),
                            Err(MenuDataRefusal::InvalidIndex)
                        );
                        let resend = |game: &mut Game<'_>| {
                            game.lan
                                .as_mut()
                                .unwrap()
                                .hub
                                .resend_menu(1, window, &game.world);
                        };
                        game.storage_scope(|game, _| {
                            assert_eq!(menu.set_data_slot(game, 0, i32::MAX), Ok(true));
                            assert!(sent(&rx).is_empty());
                            resend(game);
                            assert_eq!(
                                updates(sent(&rx)),
                                snapshot,
                                "a resend cannot see the pending write"
                            );
                            ((), false)
                        });
                        assert_eq!(menu.set_data_slot(game, 0, -80_000), Ok(false));
                        assert!(sent(&rx).is_empty());

                        game.storage_scope(|game, _| {
                            game.transactions.last_mut().unwrap().packets = Some(
                                sunrise_client::lan::conn::transaction_packets::Scope::begin(),
                            );
                            assert_eq!(menu.set_data_slot(game, 0, 100_000), Ok(true));
                            resend(game);
                            assert!(sent(&rx).is_empty());
                            ((), true)
                        });
                        assert_eq!(
                            updates(sent(&rx)),
                            [
                                snapshot[0],
                                snapshot[1],
                                DataSlot {
                                    window,
                                    index: 0,
                                    value: 100_000
                                },
                            ],
                            "the captured resend cannot roll back the committed delta"
                        );

                        let host = game as &mut dyn ServerHost;
                        let committed: Result<(), TransactionError<()>> =
                            host.transaction(|host, _| {
                                assert_eq!(menu.set_data_slot(host, 0, 1), Ok(true));
                                let nested: Result<(), TransactionError<()>> =
                                    host.transaction(|host, _| {
                                        assert_eq!(menu.set_data_slot(host, 0, i32::MIN), Ok(true));
                                        assert_eq!(menu.set_data_slot(host, 1, 5), Ok(true));
                                        Ok(())
                                    });
                                assert_eq!(nested, Ok(()));
                                assert_eq!(menu.set_data_slot(host, 1, 0), Ok(true));
                                assert!(sent(&rx).is_empty());
                                Ok(())
                            });
                        assert_eq!(committed, Ok(()));
                        assert_eq!(
                            updates(sent(&rx)),
                            [DataSlot {
                                window,
                                index: 0,
                                value: i32::MIN
                            }]
                        );
                        let state = game.lan.as_ref().unwrap().hub.guests[0]
                            .menu
                            .as_ref()
                            .unwrap()
                            .state;
                        game.lan_guest_packet(
                            1,
                            S::ContainerClick(play::ContainerClick {
                                container: window,
                                state: (state - 1) & 0x7FFF,
                                slot: -999,
                                button: 0,
                                input: sunrise_client::lan::menu::input::PICKUP,
                                changed: vec![],
                                carried: None,
                            }),
                        );
                        let packets = sent(&rx);
                        assert!(matches!(packets.first(), Some(C::ContainerSetContent(p))
                            if p.container == window && p.items.is_empty() && p.state != state
                        ));
                        assert_eq!(
                            updates(packets),
                            [
                                DataSlot {
                                    value: i32::MIN,
                                    ..snapshot[0]
                                },
                                snapshot[1]
                            ],
                            "a stale click resends every committed data slot after the contents"
                        );

                        // Force the counter to reuse the old id, as it would after 100 opens.
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |g| g.next_window = window - 1);
                        let replaced: Result<(), TransactionError<()>> =
                            (game as &mut dyn ServerHost).transaction(|host, _| {
                                assert_eq!(menu.set_data_slot(host, 0, 900), Ok(true));
                                let staged = show(host, vec![5, 6]).unwrap().unwrap();
                                assert!(staged.fields().window < 0);
                                assert_eq!(staged.container_id(host), None);
                                Ok(())
                            });
                        assert_eq!(replaced, Ok(()));
                        assert_eq!(
                            player.container_menu(game).unwrap().container_id(game),
                            Some(window)
                        );
                        assert_eq!(
                            menu.set_data_slot(game, 0, 999),
                            Err(MenuDataRefusal::Stale)
                        );
                        // The old generation's commit can update the old screen, but its
                        // data must precede OpenScreen even though the protocol id is reused.
                        let mut packets = sent(&rx);
                        let opened = packets
                            .iter()
                            .position(|p| {
                                matches!(p,
                                    C::OpenScreen(p) if p.container == window
                                )
                            })
                            .unwrap();
                        let replacement = packets.split_off(opened);
                        assert_eq!(
                            updates(packets),
                            [DataSlot {
                                window,
                                index: 0,
                                value: 900
                            }]
                        );
                        assert_eq!(
                            updates(replacement),
                            [
                                DataSlot {
                                    window,
                                    index: 0,
                                    value: 5
                                },
                                DataSlot {
                                    window,
                                    index: 1,
                                    value: 6
                                },
                            ]
                        );
                        assert!(player.container_menu(game).unwrap().close_container(game));
                        assert!(updates(sent(&rx)).is_empty());
                        assert_eq!(
                            player
                                .container_menu(game)
                                .unwrap()
                                .set_data_slot(game, 0, 1),
                            Err(MenuDataRefusal::Unsupported)
                        );

                        let old = open(game, 3, "before disconnect", MenuReplace::CloseFirst);
                        let old_window = old.container_id(game).unwrap();
                        sent(&rx);
                        disconnect.store(true, Relaxed);
                        assert_eq!(
                            show(game, vec![1, 2]),
                            Err(OpenMenuError::PlayerUnavailable)
                        );
                        let guest = game.lan.as_ref().unwrap().hub.guest(1).unwrap();
                        assert!(guest.disconnect_pending);
                        assert_eq!(guest.menu.as_ref().map(|m| m.window), Some(old_window));
                        assert!(!sent(&rx).iter().any(|packet| matches!(
                            packet,
                            C::OpenScreen(_) | C::ContainerClose(_)
                        )));
                    })
                },
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn revision_two_blockless_menu_syncs_only_encodable_indices() {
        use sunrise_client::lan::menu::input::PICKUP;
        use sunrise_protocol::menu_extra::{DATA_CHANNEL, DataSlot};

        const LAST: usize = u16::MAX as usize;
        const OVERFLOW: usize = LAST + 1;
        const FIRST_VALUE: i32 = 123_456;
        const LAST_VALUE: i32 = -80_000;
        const OVERFLOW_VALUE: i32 = 456_789;
        sunrise_client::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "status_package".into(),
                0,
                OVERFLOW + 1,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
                None,
            )],
            || {
                with_game(|game| {
                    let (rx, key) = mod_menu_guest(game);
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |g| g.sender.menu_extra_rev = 2);
                    let player = game.player(123).unwrap();
                    let mut data = vec![0; OVERFLOW + 1];
                    data[0] = FIRST_VALUE;
                    data[LAST] = LAST_VALUE;
                    data[OVERFLOW] = OVERFLOW_VALUE;
                    let mut request = ModMenuOpen::new(key, mod_api::Text::literal("status"));
                    request.data = data;
                    request.replace = MenuReplace::CloseFirst;
                    let menu = player.open_mod_menu(game, request).unwrap().unwrap();
                    let window = menu.container_id(game).unwrap();
                    let updates = || {
                        sent(&rx).into_iter().filter_map(|packet| match packet {
                            C::CustomPayload(p) if p.channel == DATA_CHANNEL => {
                                Some(DataSlot::decode(&p.data).unwrap())
                            }
                            _ => None,
                        })
                    };
                    let assert_snapshot = |route, last_value| {
                        let expected = (0..=u16::MAX).map(|index| DataSlot {
                            window,
                            index,
                            value: match index {
                                0 => FIRST_VALUE,
                                u16::MAX => last_value,
                                _ => 0,
                            },
                        });
                        assert!(
                            updates().eq(expected),
                            "incorrect revision-2 snapshot on {route}"
                        );
                    };
                    assert_snapshot("open", LAST_VALUE);
                    let _guard = mod_api::content::crash::enter_native_mod(
                        mod_api::content::crash::ModCall {
                            id: "status_package",
                            hook: "test",
                            part: None,
                        },
                    );
                    assert_eq!(player.container_menu(game), Some(menu));
                    let host: &mut dyn ServerHost = game;
                    let committed: Result<(), mod_api::TransactionError<()>> =
                        host.transaction(|host, _| {
                            assert_eq!(menu.set_data_slot(host, LAST, i32::MAX), Ok(true));
                            assert!(sent(&rx).is_empty());
                            Ok(())
                        });
                    assert_eq!(committed, Ok(()));
                    assert!(updates().eq([DataSlot {
                        window,
                        index: u16::MAX,
                        value: i32::MAX,
                    }]));
                    let committed: Result<(), mod_api::TransactionError<()>> =
                        host.transaction(|host, _| {
                            assert_eq!(menu.set_data_slot(host, OVERFLOW, i32::MIN), Ok(true));
                            assert!(sent(&rx).is_empty());
                            Ok(())
                        });
                    assert_eq!(committed, Ok(()));
                    assert!(sent(&rx).is_empty(), "unencodable index sent a delta");

                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    let open = hub.guests[0].menu.as_ref().unwrap();
                    assert_eq!(open.blockless_data[OVERFLOW], i32::MIN);
                    hub.resend_menu(1, window, &game.world);
                    assert_snapshot("explicit resend", i32::MAX);
                    let state = hub.guests[0].menu.as_ref().unwrap().state;
                    // The live native click route resends in prepare_click -> answer_click.
                    game.lan_guest_packet(
                        1,
                        S::ContainerClick(play::ContainerClick {
                            container: window,
                            state: (state - 1) & 0x7FFF,
                            slot: -999,
                            button: 0,
                            input: PICKUP,
                            changed: vec![],
                            carried: None,
                        }),
                    );
                    assert_snapshot("stale click", i32::MAX);
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn staged_same_level_open_spills_the_old_chest_before_its_veto_replaces_it() {
        use mod_api::{BlockState, BlockUpdateFlags, ContainerOpenedEvent, Flow, HostBlockStates};
        use sunrise_client::chunk::Chunk;

        const POS: [i32; 3] = [2, 64, 2];
        const CELL: (i32, i32, i32) = (POS[0], POS[1], POS[2]);
        const CONTENT: &str = "minecraft:gold_ingot";
        const COUNT: u32 = 7;
        struct Veto;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn container_opened(
                &self,
                host: &mut dyn ServerHost,
                event: &mut ContainerOpenedEvent<'_>,
            ) -> Flow {
                assert_eq!(event.kind, MOD_MENU);
                assert!(host.transaction_matches(None));
                let level = host.level("minecraft:overworld").unwrap();
                let chest = BlockState::of(
                    &host.block_states(),
                    mod_api::__private::block_key("minecraft:chest").unwrap(),
                    &[],
                )
                .unwrap();
                assert_eq!(
                    level.set_block(host, POS, chest, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                Flow::Cancel
            }
        }
        let mods = vec![mod_api::ModEntry::new("replace_chest_veto", Box::new(Veto))].into();
        mod_platform::testing::with_mods(mods, || {
            with_mod_menu_game(|game| {
                let (_rx, key) = mod_menu_guest(game);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                let chest = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:chest")
                    .unwrap();
                assert_eq!(
                    game.level_set_block(level, POS, chest, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                game.world
                    .block_entities
                    .get_mut(&CELL)
                    .unwrap()
                    .slots_mut()[0] = Some(ItemStack::new(CONTENT, COUNT));
                let player = game.player(123).unwrap();
                (game as &mut dyn ServerHost)
                    .transaction(|host, _| {
                        let staged = player
                            .open_mod_menu(
                                host,
                                ModMenuOpen::new(key, mod_api::Text::literal("vetoed")),
                            )
                            .unwrap()
                            .unwrap();
                        assert!(staged.fields().window < 0);
                        let air = BlockState::of(
                            &host.block_states(),
                            mod_api::__private::block_key("minecraft:air").unwrap(),
                            &[],
                        )
                        .unwrap();
                        assert_eq!(
                            level.set_block(host, POS, air, BlockUpdateFlags::NEIGHBORS),
                            Ok(true)
                        );
                        Ok::<_, ()>(())
                    })
                    .unwrap();

                assert_eq!(game.world.block(CELL.0, CELL.1, CELL.2), chest);
                let new_chest = game.world.block_entities.get(&CELL).unwrap();
                assert!(new_chest.slots().iter().all(Option::is_none));
                let dropped: u32 = game
                    .dropped
                    .iter()
                    .filter(|entity| &*entity.stack.item == CONTENT)
                    .map(|entity| entity.stack.count)
                    .sum();
                assert_eq!(
                    dropped, COUNT,
                    "the old chest must spill its entire inventory"
                );
                assert!(
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(1)
                        .unwrap()
                        .menu
                        .is_none()
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn cross_dimension_mod_menu_open_in_transaction_isolates_veto_writes() {
        use super::super::Dim;
        use mod_api::{BlockUpdateFlags, ContainerOpenedEvent, HostBlockStates};
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        use sunrise_client::chunk::Chunk;

        const POS: [i32; 3] = [2, 64, 2];
        struct Veto {
            calls: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn container_opened(
                &self,
                host: &mut dyn ServerHost,
                _: &mut ContainerOpenedEvent<'_>,
            ) -> mod_api::Flow {
                self.calls.fetch_add(1, Relaxed);
                assert!(
                    host.transaction_matches(None),
                    "deferred cross-level veto must not borrow the caller's transaction journal"
                );
                let nether = host.level("minecraft:the_nether").unwrap();
                let gold = mod_api::BlockState::of(
                    &host.block_states(),
                    mod_api::__private::block_key("minecraft:gold_block").unwrap(),
                    &[],
                )
                .unwrap();
                assert_eq!(
                    nether.set_block(host, POS, gold, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                mod_api::Flow::Continue
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cross_veto",
            Box::new(Veto {
                calls: calls.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_client::containers::with_mod_menu_types(
                [(
                    MOD_MENU.into(),
                    "cross_veto".into(),
                    1,
                    0,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    true,
                    None,
                )],
                || {
                    with_game(|game| {
                        let (_rx, key) = mod_menu_guest(game);
                        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                            g.dimension = Dim::Nether.id().to_owned();
                        });
                        let registry = game.pack.assets.registry;
                        let stone = registry.default_state("minecraft:stone").unwrap();
                        let netherrack = registry.default_state("minecraft:netherrack").unwrap();
                        let cobblestone = registry.default_state("minecraft:cobblestone").unwrap();
                        let gold_block = registry.default_state("minecraft:gold_block").unwrap();

                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                        game.world.set(POS[0], POS[1], POS[2], stone).unwrap();
                        game.enter_level(Dim::Nether).unwrap();
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                        game.world.set(POS[0], POS[1], POS[2], netherrack).unwrap();
                        game.enter_level(Dim::Overworld).unwrap();

                        let player = game.player(123).unwrap();
                        let aborted: Result<(), mod_api::TransactionError<()>> =
                            (game as &mut dyn ServerHost).transaction(|host, _| {
                                let overworld = host.level("minecraft:overworld").unwrap();
                                let planks = mod_api::BlockState::of(
                                    &host.block_states(),
                                    mod_api::__private::block_key("minecraft:oak_planks").unwrap(),
                                    &[],
                                )
                                .unwrap();
                                assert_eq!(
                                    overworld.set_block(
                                        host,
                                        POS,
                                        planks,
                                        BlockUpdateFlags::NEIGHBORS,
                                    ),
                                    Ok(true)
                                );
                                let staged = player
                                    .open_mod_menu(
                                        host,
                                        ModMenuOpen::new(key, mod_api::Text::literal("aborted")),
                                    )
                                    .unwrap()
                                    .unwrap();
                                assert!(staged.fields().window < 0);
                                assert_eq!(calls.load(Relaxed), 0);
                                Err(())
                            });
                        assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
                        assert_eq!(game.dim, Dim::Overworld);
                        assert_eq!(calls.load(Relaxed), 0);
                        assert_eq!(game.world.block(POS[0], POS[1], POS[2]), stone);
                        assert!(
                            game.lan
                                .as_ref()
                                .unwrap()
                                .hub
                                .guest(1)
                                .unwrap()
                                .menu
                                .is_none()
                        );
                        game.enter_level(Dim::Nether).unwrap();
                        assert_eq!(game.world.block(POS[0], POS[1], POS[2]), netherrack);
                        game.enter_level(Dim::Overworld).unwrap();

                        let committed: Result<(), mod_api::TransactionError<()>> =
                            (game as &mut dyn ServerHost).transaction(|host, _| {
                                let overworld = host.level("minecraft:overworld").unwrap();
                                let cobble = mod_api::BlockState::of(
                                    &host.block_states(),
                                    mod_api::__private::block_key("minecraft:cobblestone").unwrap(),
                                    &[],
                                )
                                .unwrap();
                                assert_eq!(
                                    overworld.set_block(
                                        host,
                                        POS,
                                        cobble,
                                        BlockUpdateFlags::NEIGHBORS,
                                    ),
                                    Ok(true)
                                );
                                let staged = player
                                    .open_mod_menu(
                                        host,
                                        ModMenuOpen::new(key, mod_api::Text::literal("committed")),
                                    )
                                    .unwrap()
                                    .unwrap();
                                assert!(staged.fields().window < 0);
                                assert_eq!(calls.load(Relaxed), 0);
                                Ok(())
                            });
                        assert_eq!(committed, Ok(()));
                        assert_eq!(game.dim, Dim::Overworld);
                        assert_eq!(calls.load(Relaxed), 1);
                        assert_eq!(game.world.block(POS[0], POS[1], POS[2]), cobblestone);
                        let live = game
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(1)
                            .unwrap()
                            .menu
                            .as_ref()
                            .expect("committed cross-level open installed the menu")
                            .window;
                        assert!((1..=100).contains(&live));
                        game.enter_level(Dim::Nether).unwrap();
                        assert_eq!(game.world.block(POS[0], POS[1], POS[2]), gold_block);
                        game.enter_level(Dim::Overworld).unwrap();
                    });
                },
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn aborted_staged_menu_opens_never_wrap_onto_or_access_the_live_window() {
        use mod_api::{MenuItemsRefusal, MenuSlotBackRefusal, PlayerContainerOpenEvent};
        use std::sync::Mutex;

        struct Opened(Arc<Mutex<Vec<Option<i32>>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Opened {
            fn player_container_open(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                self.0
                    .lock()
                    .unwrap()
                    .push(event.menu.and_then(|menu| menu.container_id(host)));
            }
        }
        let opened = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "opened",
            Box::new(Opened(opened.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_client::containers::with_mod_menu_types(
                [(
                    MOD_MENU.into(),
                    "opened".into(),
                    1,
                    0,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    true,
                    None,
                )],
                || {
                    with_game(|game| {
                        let (rx, key) = mod_menu_guest(game);
                        let player = game.player(123).unwrap();
                        let stone = ItemStack::new("minecraft:stone", 1).try_into_api().unwrap();
                        let dirt = ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap();

                        for start_counter in [0, 99] {
                            game.lan
                                .as_mut()
                                .unwrap()
                                .hub
                                .guests
                                .update(0, |g| g.next_window = start_counter);
                            let old = open(game, 1, "live", MenuReplace::CloseFirst);
                            let live = old.container_id(game).unwrap();
                            assert_eq!(live, start_counter % 100 + 1);
                            old.set_items(game, vec![Some(stone.clone())]).unwrap();
                            opened.lock().unwrap().clear();
                            sent(&rx);

                            for step in 0..105 {
                                let aborted: Result<(), mod_api::TransactionError<()>> =
                                    (game as &mut dyn ServerHost).transaction(|host, _| {
                                        let staged = if step % 2 == 0 {
                                            player
                                                .open_chest_menu(
                                                    host,
                                                    1,
                                                    mod_api::Text::literal("aborted"),
                                                    vec![Some(dirt.clone())],
                                                    MenuReplace::InPlace,
                                                )
                                                .unwrap()
                                        } else {
                                            let mut req = ModMenuOpen::new(
                                                key,
                                                mod_api::Text::literal("aborted mod"),
                                            );
                                            req.replace = MenuReplace::InPlace;
                                            player.open_mod_menu(host, req).unwrap().unwrap()
                                        };
                                        assert!(staged.fields().window < 0);
                                        assert_ne!(staged, old);
                                        assert_eq!(staged.container_id(host), None);
                                        assert_eq!(
                                            staged.set_items(host, vec![Some(dirt.clone())]),
                                            Err(MenuItemsRefusal::NotOpen)
                                        );
                                        assert_eq!(
                                            staged.back_slots(host, Vec::new()),
                                            Err(MenuSlotBackRefusal::NoPlayerOrWindow)
                                        );
                                        assert_eq!(player.container_menu(host), Some(old));
                                        Err(())
                                    });
                                assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
                            }

                            assert_eq!(
                                game.lan.as_ref().unwrap().hub.guest(1).unwrap().next_window,
                                live
                            );
                            assert_eq!(old.slot(game, 0), Some(Some(stone.clone())));
                            assert!(opened.lock().unwrap().is_empty());
                            assert!(sent(&rx).is_empty());

                            let committed: Result<MenuRef, mod_api::TransactionError<()>> =
                                (game as &mut dyn ServerHost).transaction(|host, _| {
                                    let staged = player
                                        .open_chest_menu(
                                            host,
                                            1,
                                            mod_api::Text::literal("replacement"),
                                            vec![Some(dirt.clone())],
                                            MenuReplace::InPlace,
                                        )
                                        .unwrap();
                                    assert!(staged.fields().window < 0);
                                    assert_eq!(staged.container_id(host), None);
                                    assert_eq!(
                                        staged.set_items(host, vec![Some(dirt.clone())]),
                                        Err(MenuItemsRefusal::NotOpen)
                                    );
                                    Ok(staged)
                                });
                            let staged = committed.unwrap();
                            let next = live % 100 + 1;
                            assert_ne!(next, live);
                            assert_eq!(*opened.lock().unwrap(), [Some(next)]);
                            let replacement = player.container_menu(game).unwrap();
                            assert_eq!(replacement.container_id(game), Some(next));
                            assert_eq!(old.container_id(game), None);
                            assert_eq!(staged.container_id(game), None);
                            assert_eq!(replacement.slot(game, 0), Some(Some(dirt.clone())));
                            let packets = sent(&rx);
                            assert!(
                                packets.iter().any(|p| {
                                    matches!(p, C::OpenScreen(p) if p.container == next)
                                })
                            );
                            assert!(!packets.iter().any(|p| matches!(p, C::ContainerClose(_))));

                            // An in-flight click for the outgoing window is ignored by the replacement.
                            game.lan_guest_packet(
                                1,
                                S::ContainerClick(play::ContainerClick {
                                    container: live,
                                    state: 0,
                                    slot: 0,
                                    button: 0,
                                    input: sunrise_client::lan::menu::input::PICKUP,
                                    changed: vec![],
                                    carried: None,
                                }),
                            );
                            assert!(sent(&rx).is_empty());
                            assert_eq!(replacement.slot(game, 0), Some(Some(dirt.clone())));
                        }
                    });
                },
            );
        });
    }

    #[test]
    fn menu_clicks_map_to_the_inventory_below() {
        // A 6-row chest: 54 slots, then the inventory's 27, then the hotbar.
        assert_eq!(native_menu_inventory_slot(54, 3), None);
        assert_eq!(native_menu_inventory_slot(54, 54), Some(9));
        assert_eq!(native_menu_inventory_slot(54, 80), Some(35));
        assert_eq!(native_menu_inventory_slot(54, 81), Some(0));
        assert_eq!(native_menu_inventory_slot(54, 89), Some(8));
        assert_eq!(native_menu_inventory_slot(54, 90), None);
        assert_eq!(native_menu_inventory_slot(54, -999), None);
        assert_eq!(native_menu_inventory_slot(9, 9), Some(9));
    }
}

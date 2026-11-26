//! Private custom-book synchronization, admitted by its own configuration capability.
use super::Game;
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicU64, Ordering},
};
use sunrise_client::recipe_book::{
    placement_sync::ResourceId,
    settings_codec,
    settings_sync::{CHANNEL, Message},
};
use sunrise_protocol::packets::{
    configuration::CustomPayload,
    play::{Clientbound as C, Serverbound as S},
};
static EPOCH: AtomicU64 = AtomicU64::new(1);

pub(super) fn book_payload(message: &Message) -> Option<CustomPayload> {
    match settings_codec::encode(message) {
        Ok(data) => Some(CustomPayload {
            channel: CHANNEL.into(),
            data,
        }),
        Err(error) => {
            log::warn!("[recipe book] cannot send {CHANNEL}: {error}");
            None
        }
    }
}

fn registered() -> BTreeSet<ResourceId> {
    mod_api::RecipeBookTypeKey::registered()
        .into_iter()
        .map(|key| ResourceId::from_registered_book(&key))
        .collect()
}
impl Game<'_> {
    pub(super) fn sync_replaced_book_settings(&mut self, uuid: u128) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(guest) = host.hub.guests.iter().find(|guest| guest.uuid == uuid) else {
            return;
        };
        let Some(authority) = self
            .native_net
            .book_connections
            .get_mut(&guest.conn)
            .and_then(|connection| connection.authority.as_mut())
        else {
            return;
        };
        let Some(revision) = authority.revision.checked_add(1) else {
            return;
        };
        authority.revision = revision;
        if let Some(payload) = book_payload(&authority.snapshot(&guest.recipe_book.custom)) {
            guest.send(&C::CustomPayload(payload));
        }
    }

    pub(super) fn reset_book_view(&mut self) {
        self.native_net.placement.client = Default::default();
        self.clear_mod_book_placement_screen();
        self.native_net.book_view = Default::default();
        self.native_net.book_request = 0;
        self.native_net.book_ready = false;
        self.build_screen();
    }
    pub(super) fn clear_guest_book_connection(&mut self, conn: u32) {
        self.guest_combat.remove(&conn);
        self.native_net.book_connections.remove(&conn);
        self.native_net.placement.servers.remove(&conn);
    }
    pub(super) fn notify_vanilla_book_settings(
        &mut self,
        uuid: u128,
        name: &str,
        book: i32,
        open: bool,
        filtering: bool,
    ) {
        let book = match book {
            0 => mod_api::RecipeBookIdentity::Crafting,
            1 => mod_api::RecipeBookIdentity::Furnace,
            2 => mod_api::RecipeBookIdentity::BlastFurnace,
            3 => mod_api::RecipeBookIdentity::Smoker,
            _ => return,
        };
        self.native_player_recipe_book_settings_change(uuid, name, book, open, filtering);
    }

    /// Authoritative view for a custom menu panel. None disables the panel
    /// until negotiation and the initial snapshot finish.
    pub fn custom_book_settings(
        &self,
        book: &mod_api::RecipeBookTypeKey,
    ) -> Option<mod_api::RecipeBookSettings> {
        let view = &self.native_net.book_view;
        if !view.ready || !view.contains(book.as_str()) {
            return None;
        }
        let flags = view.books.get(book.as_str());
        Some(mod_api::RecipeBookSettings {
            open: flags.open,
            filtering: flags.filtering,
        })
    }
    /// Queue intent without changing the authoritative or saved pair.
    pub fn request_custom_book_settings(
        &mut self,
        book: &mod_api::RecipeBookTypeKey,
        flags: mod_api::RecipeBookSettings,
    ) -> bool {
        if self.custom_book_settings(book).is_none() {
            return false;
        }
        let Some(number) = self.native_net.book_request.checked_add(1) else {
            return false;
        };
        let Some(epoch) = self.native_net.book_view.epoch else {
            return false;
        };
        let message = Message::Request {
            epoch,
            number,
            key: ResourceId::from_registered_book(book),
            flags: sunrise_client::recipe_book::BookFlags {
                open: flags.open,
                filtering: flags.filtering,
            },
        };
        let Some(payload) = book_payload(&message) else {
            return false;
        };
        self.native_net.book_request = number;
        self.send(S::CustomPayload(payload));
        true
    }

    pub(super) fn sync_server_book_write(
        &mut self,
        player: mod_api::PlayerRef,
        book: &mod_api::RecipeBookTypeKey,
        flags: sunrise_client::recipe_book::BookFlags,
    ) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(guest) = host
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == player.uuid() && g.placed)
        else {
            return;
        };
        let Some(authority) = self
            .native_net
            .book_connections
            .get_mut(&guest.conn)
            .and_then(|connection| connection.authority.as_mut())
        else {
            return;
        };
        if let Some(message) = authority.server_update(book.as_str(), flags)
            && let Some(payload) = book_payload(&message)
        {
            guest.send(&C::CustomPayload(payload));
        }
    }

    pub(super) fn begin_book_negotiation(&mut self) {
        self.reset_book_view();
        if self
            .remote
            .as_ref()
            .is_some_and(|r| r.custom_books && r.joined)
            && let Some(payload) = book_payload(&Message::Hello)
        {
            self.send(S::CustomPayload(payload));
        }
    }
    pub(super) fn guest_book_settings(&mut self, conn: u32, data: &[u8]) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if !host.hub.guest(conn).is_some_and(|guest| guest.placed) {
            return;
        }
        let connection = self.native_net.book_connections.entry(conn).or_default();
        if data.first() == Some(&settings_codec::PLACEMENT_TAG) {
            let frame = connection.receive_placement(data);
            if let Some(frame) = frame {
                self.guest_mod_book_placement(conn, frame);
            }
            return;
        }
        let changed = host
            .hub
            .guests
            .update_connection(conn, |guest| {
                let reply =
                    connection.receive(data, &mut guest.recipe_book.custom, registered, || {
                        EPOCH.fetch_add(1, Ordering::Relaxed)
                    })?;
                if let Some(payload) = book_payload(&reply.message) {
                    guest.send(&C::CustomPayload(payload));
                }
                reply
                    .changed
                    .map(|(id, flags)| (guest.uuid, guest.name.clone(), id, flags))
            })
            .flatten();
        if let Some((uuid, name, id, flags)) = changed
            && let Some(key) = mod_api::RecipeBookTypeKey::registered()
                .into_iter()
                .find(|key| key.as_str() == id.as_str())
        {
            self.native_player_recipe_book_settings_change(
                uuid,
                &name,
                mod_api::RecipeBookIdentity::Custom(key),
                flags.open,
                flags.filtering,
            );
        }
    }
    pub(super) fn remote_book_settings(&mut self, data: &[u8]) {
        if !self
            .remote
            .as_ref()
            .is_some_and(|remote| remote.custom_books && remote.joined)
        {
            return;
        }
        let Some(message) = settings_codec::decode(data) else {
            log::warn!(
                "[recipe book] ignored malformed settings payload ({} bytes)",
                data.len()
            );
            return;
        };
        if let Message::Placement(frame) = message {
            if self.native_net.book_ready && self.native_net.book_view.ready {
                self.remote_mod_book_placement(frame);
            } else {
                log::warn!("[recipe book] ignored placement frame before settings sync completed");
            }
            return;
        }
        if message == Message::Ready {
            if self.native_net.book_ready {
                log::warn!("[recipe book] ignored duplicate settings ready");
                return;
            }
            self.native_net.book_ready = true;
            if let Some(payload) = book_payload(&Message::Offer(registered().into_iter().collect()))
            {
                self.send(S::CustomPayload(payload));
            }
            return;
        }
        if !self.native_net.book_ready
            || !matches!(message, Message::Snapshot { .. } | Message::Update { .. })
        {
            log::warn!(
                "[recipe book] ignored unexpected clientbound settings message: {message:?}"
            );
            return;
        }
        let previous_epoch = self.native_net.book_view.epoch;
        if !self.native_net.book_view.receive(message, &registered()) {
            log::warn!("[recipe book] ignored stale or invalid settings update");
            return;
        }
        if previous_epoch != self.native_net.book_view.epoch {
            self.native_net.placement.client = Default::default();
            self.clear_mod_book_placement_screen();
        }
        self.build_screen();
        if self.native_net.book_view.ready {
            self.begin_mod_book_placement();
            self.ensure_mod_book_catalog();
        }
    }
}

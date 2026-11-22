//! The window's connection to a server, as the engine sees it: its record of the last server, the
//! hooks it calls as a connection starts, lets the player in and closes (`sunrise_hooks::connection`),
//! and the Disconnected screen's offer to connect again, which a crate above asks for
//! (`Hooks::retry`). With nothing asking, the screen is vanilla's `DisconnectedScreen`.
//!
//! A connection over the network is one session, from `connection_starts` until it closes, once,
//! in `connection_left` or `connection_ended`, whichever way: the player leaving, the game closing,
//! a transfer, the server, the connection or this client ending it. A world the window plays
//! itself never opens one.
use super::*;
use sunrise_client::{multiplayer, title::Mode};
use sunrise_hooks::connection::{By, Ended, Left, Retry, Server};

impl Game<'_> {
    /// A connection to `address` as `name` starts over the network (`Hooks::connecting`). One
    /// still open closes first, as the player leaving it.
    pub(in crate::game) fn connection_starts(&mut self, address: &str, name: &str, lan: bool) {
        self.connection_left(Left::Disconnected);
        self.pending_retry = None;
        self.retry_due = None;
        let server = Server::new(address, name, lan);
        sunrise_hooks::engine::connecting(&server);
        self.last_connection = Some(server);
        self.connection_open = true;
    }
    /// The server this window connected to last over the network: while connected, and after a
    /// lost connection while the Disconnected screen may connect there again.
    pub(in crate::game) fn server_address(&self) -> Option<&str> {
        let server = self.last_connection.as_ref();
        server.map(|server| server.address.as_str())
    }
    /// That server was found on the local network.
    pub(in crate::game) fn server_is_lan(&self) -> bool {
        self.last_connection
            .as_ref()
            .is_some_and(|server| server.lan)
    }
    /// The server let the player in (`Hooks::joined`); nothing for a world the window plays
    /// itself, whose session joins the same way.
    pub(in crate::game) fn connection_joined(&mut self) {
        if !self.connection_open {
            return;
        }
        sunrise_hooks::engine::joined();
    }
    /// The open network connection closes without being lost (`Hooks::left`). Nothing when
    /// none is open: it closed already, or the window plays its own world.
    pub(in crate::game) fn connection_left(&mut self, how: Left) {
        if !std::mem::take(&mut self.connection_open) {
            return;
        }
        self.last_connection = None;
        sunrise_hooks::engine::left(how);
    }
    /// The open network connection ended without the player leaving (`Hooks::ended`), before
    /// the Disconnected screen shows; nothing when none is open. The crates are asked once
    /// whether to connect again (`Hooks::retry`), and that screen acts on the answer
    /// (`offer_retry`); the last server is forgotten unless something will connect there again.
    pub(in crate::game) fn connection_ended(&mut self, reason: &str, key: Option<&str>, by: By) {
        if !std::mem::take(&mut self.connection_open) {
            return;
        }
        sunrise_hooks::engine::ended(&Ended::new(reason, key, by));
        self.pending_retry = sunrise_hooks::engine::retry();
        if self.pending_retry.is_none() {
            self.last_connection = None;
        }
    }
    /// The Disconnected screen just shown offers to connect again when the answer to the
    /// connection's end asked to: its line, or the connection on the next frame (`follow_retry`)
    /// when it is due at once.
    pub(in crate::game) fn offer_retry(&mut self) {
        let Some(answer) = self.pending_retry.take() else {
            return;
        };
        if retry_slot(&mut self.title).is_none() {
            return;
        }
        let line = match answer {
            Retry::Wait(line) => line,
            Retry::Now(server) => {
                self.retry_due = Some(server);
                String::new()
            }
            // An answer this engine doesn't know: no offer.
            _ => return,
        };
        if let Some(server) = &self.last_connection {
            let address = sunrise_client::net::shown_address(&server.address);
            log::info!("Sunrise: offering to connect to {address} again: {line}");
        }
        if let Some(slot) = retry_slot(&mut self.title) {
            *slot = Some(line);
        }
    }
    /// Once a frame while the Disconnected screen offers to connect again: the connection due
    /// from the last answer, else the crates' answer now (its line, the connection, or nothing
    /// any more). Cancelled or left, the offer and its due connection go. An online-mode server
    /// gets the signed-in account again: `connect_server` takes it from `auth::current()`, held
    /// in memory for the whole run (the launcher's session file is gone after the first read).
    pub(in crate::game) fn follow_retry(&mut self) -> Result<()> {
        if retry_slot(&mut self.title).is_none_or(|slot| slot.is_none()) {
            self.retry_due = None;
            return Ok(());
        }
        let answer = match self.retry_due.take() {
            Some(server) => Some(Retry::Now(server)),
            None => sunrise_hooks::engine::retry(),
        };
        let Some(slot) = retry_slot(&mut self.title) else {
            return Ok(());
        };
        let server = match answer {
            Some(Retry::Wait(line)) => {
                *slot = Some(line);
                return Ok(());
            }
            Some(Retry::Now(server)) => server,
            // Nothing asks any more.
            _ => {
                *slot = None;
                return Ok(());
            }
        };
        *slot = None;
        let realm = self.realm_session && self.server_address() == Some(server.address.as_str());
        self.connect_server_with_options(&server.address, &server.name, None, server.lan, realm)
    }
}

/// The Disconnected screen's line about connecting again, while that screen shows.
fn retry_slot(title: &mut Option<sunrise_client::title::Title>) -> Option<&mut Option<String>> {
    match &mut title.as_mut()?.mode {
        Mode::Multiplayer(multiplayer::Screen::Disconnected { retry, .. }) => Some(retry),
        _ => None,
    }
}

impl Game<'_> {
    /// Leave the title for a server (ConnectScreen): an empty world fed by the connection.
    pub(in crate::game) fn connect_server(&mut self, address: &str, name: &str) -> Result<()> {
        self.connect_server_with_options(address, name, None, false, false)
    }
    /// `ConnectScreen.startConnecting`: keep LAN discovery's server kind through loading.
    pub(in crate::game) fn connect_server_with_options(
        &mut self,
        address: &str,
        name: &str,
        cookies: Option<sunrise_client::net::Cookies>,
        lan: bool,
        realm: bool,
    ) -> Result<()> {
        crate::game::world_loading::world_data_used();
        if self.defer_world_entry(crate::game::PendingWorldEntry::Server {
            address: address.to_owned(),
            name: name.to_owned(),
            cookies: cookies.clone(),
            lan,
            realm,
        })? {
            return Ok(());
        }
        self.join_world_rest()?;
        self.client_logging_out();
        if let Err(error) = self.join_server(address, name, None, cookies, lan, realm) {
            log::warn!("Sunrise: could not join server: {error}");
            let skip = self.settings.file.bool("skipMultiplayerWarning");
            match &mut self.title {
                Some(title) => {
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                }
                None => {
                    let mut title = self
                        .new_title(std::env::current_dir()?.join("saves"))
                        .with_user(|| self.options.player_name());
                    title.skip_multiplayer_warning = skip;
                    title.failed(error);
                    self.show_title(title);
                }
            }
            self.capture(false);
        }
        Ok(())
    }
    /// A stack to the server (`SetCreativeModeSlot`): the same component
    /// patch a Sunrise server sends its clients, with the registries' ids, so
    /// the server's echo doesn't hand back a bare item.
    pub(super) fn to_network(&self, s: &Option<ItemStack>) -> Option<Stack> {
        self.to_network_projecting(s, &|_| ())
    }
    // handle_command_suggestions: remote/commands.rs
    pub(in crate::game) fn set_registries(&mut self, registries: Arc<Dynamic>) {
        let entries: Vec<(&str, serde_json::Value)> = registries
            .registries
            .get("minecraft:enchantment")
            .into_iter()
            .flatten()
            .filter_map(|(name, data)| {
                Some((
                    name.as_str(),
                    sunrise_client::scoreboard::tag_to_json(data.as_ref()?),
                ))
            })
            .collect();
        let enchantments = match self
            .stations()
            .enchantments
            .with_entries(entries.iter().map(|(n, v)| (*n, v)))
        {
            Ok(table) => table.map(Arc::new),
            Err(error) => {
                log::warn!("Sunrise: refused enchantment registry: {error}");
                return;
            }
        };
        // The server's biomes the game lacks (a world data pack's) join the
        // game's table, from the data it sent: colours, meshing and the
        // chunk format all go through that table. The window's own server
        // thread set the world's table already.
        if self.integrated.is_none() {
            let table = registries
                .registries
                .get(BIOME_REGISTRY)
                .and_then(|list| client_biomes(self.res.biomes(), list))
                .map(Arc::new);
            if table.is_some() || self.world_biomes.is_some() {
                self.world_biomes_base = table.clone();
                self.world_biomes = table;
                let context = self.worker_context();
                self.streaming.reload_resources(&mut self.world, context);
            }
        }
        let biomes: Vec<u16> = registries
            .registries
            .get(BIOME_REGISTRY)
            .map(|list| {
                list.iter()
                    .map(|(name, _)| world_biomes!(self).id(name))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(r) = self.remote.as_mut() {
            r.enchantments = enchantments;
            let bits = ceil_log2(biomes.len());
            let column = r.chunk_format.layout();
            r.chunk_format =
                Arc::new(ChunkFormat::new(&r.dimension, biomes, bits).in_column(column));
            r.registries = registries;
            r.resolve_clocks();
            self.install_server_items();
            self.reset_creative_contents();
            self.refresh_creative_search_tags();
        }
    }
    /// Another player of this level, whose copy passes every click (`Player.interactOn`).
    pub(in crate::game) fn is_level_player(&self, id: i32) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|r| r.level_players.contains_key(&id))
    }
    /// `ConnectScreen`'s Cancel: a server's (single player's server isn't
    /// joined through `ConnectScreen`), until the login is done.
    pub(super) fn connect_cancel(&self, (w, h): (f32, f32)) -> Option<(f32, f32)> {
        let r = self.remote.as_ref()?;
        (self.player.is_none() && !r.joined && self.integrated.is_none())
            .then(|| sunrise_client::level_loading::connect_cancel(w, h))
    }
    /// A click on `ConnectScreen`: Cancel aborts the connection
    /// (`connect.aborted`) and goes back. Escape does nothing there
    /// (`shouldCloseOnEsc` is false).
    pub(in crate::game) fn connect_screen_click(&mut self) -> Result<bool> {
        // ServerReconfigScreen's Disconnect, once it is active.
        if let Some(since) = self.remote.as_ref().and_then(|r| r.reconfiguring) {
            let (_, (x, y)) = self.reconfigure_layout(self.gui_size());
            if self.ticks.saturating_sub(since) >= 600
                && sunrise_client::ui::hovered(self.cursor, x, y, 150.0, 20.0)
            {
                self.audio.play_button_click();
                self.leave_server(None)?;
                return Ok(true);
            }
            return Ok(false);
        }
        let Some((x, y)) = self.connect_cancel(self.gui_size()) else {
            return Ok(false);
        };
        if sunrise_client::ui::hovered(self.cursor, x, y, 200.0, 20.0) {
            self.audio.play_button_click();
            self.leave_server(None)?;
            return Ok(true);
        }
        Ok(false)
    }
}

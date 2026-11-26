//! The world's save on a timer (`MinecraftServer.computeNextAutosaveInterval`,
//! `onTickRateChanged`), the players' autosaves, and the world's metadata (level.dat).
use super::*;

impl Game<'_> {
    pub(super) fn admit_entity_save() -> Result<()> {
        if !sunrise_client::join_seq::visibility::passes_settled() {
            return Err("entity pass is lifted".into());
        }
        Ok(())
    }

    pub(super) fn save_game(&mut self) -> Result<()> {
        self.save_game_with(true)
    }

    /// `MinecraftServer.computeNextAutosaveInterval`: three hundred seconds
    /// of ticks at the rate the server runs at now - its measured rate while
    /// a `/tick sprint` runs - never fewer than a hundred.
    pub(super) fn next_autosave_interval(&self) -> i64 {
        let rate = if self.clock.sprinting() {
            1.0e9 / (self.clock.average_nanos() + 1) as f32
        } else {
            self.clock.rate()
        };
        autosave_interval(rate)
    }

    /// `ServerTickRateManager.setTickRate`: set the rate, run the hook and autosave
    /// adjustment, then send the current ticking state to clients.
    pub(in crate::game) fn set_tick_rate(&mut self, rate: f32) -> bool {
        if !self.is_logical_server() || !rate.is_finite() {
            let mod_id = mod_api::content::crash::current_native_mod().unwrap_or("unknown");
            log::warn!(
                "mod {mod_id}: set_tick_rate refused {rate}: requires a finite rate on the logical server"
            );
            return false;
        }
        self.clock.set_rate(rate);
        self.on_tick_rate_changed();
        // A listener can retune the clock, so send its state after the notification.
        if let Some(host) = &mut self.lan {
            host.hub
                .send_ticking_state(self.clock.rate(), self.clock.frozen());
        }
        true
    }

    /// `MinecraftServer.onTickRateChanged`: server mods hear of it, then a rate that
    /// asks for a shorter wait than the one already running shortens it.
    pub(super) fn on_tick_rate_changed(&mut self) {
        if self.is_logical_server() {
            self.native_server_tick_rate_changed();
        }
        let interval = self.next_autosave_interval();
        if interval < self.ticks_until_autosave {
            self.ticks_until_autosave = interval;
        }
    }

    pub(super) fn finish_player_autosaves(&mut self, wait: bool) -> Result<bool> {
        let saved = self.player_autosaves.completed(wait);
        if !saved.is_empty() {
            let directory = sunrise_client::lan::playerdata::directory(&self.world_root);
            for player in saved {
                self.native_player_save_to_file(player.uuid, &player.name, &directory);
            }
        }
        self.player_autosaves.writes_finished()
    }

    pub(super) fn save_game_metadata(&mut self) -> Result<()> {
        let owner_saved = self.save_placed_players();
        self.save_world_metadata(owner_saved)
    }

    /// `PlayerList.saveAll`, without removing players or their vehicles. Finish the
    /// player writes and their file events before `ServerLevel.save` can notify mods.
    fn save_placed_players(&mut self) -> bool {
        let mut owner_saved = false;
        if self.options.execution_role.writes_singleplayer_player()
            && !self.crash_save.keep_owner_file
        {
            let stored_owner = self.store_owner();
            // A local player can be present in an offline game without a network player list.
            if stored_owner.is_some() || (self.lan.is_none() && self.player.is_some()) {
                if stored_owner.is_none() {
                    self.sync_level();
                }
                // A synchronous owner write must not overtake an older player snapshot.
                let _ = self.finish_player_autosaves(true);
                match self.level.write_player(&self.world_root, self.res.items()) {
                    Ok(()) => {
                        owner_saved = true;
                        self.save_stats();
                        self.save_advancements();
                        if let Some((uuid, name)) = stored_owner {
                            let directory =
                                sunrise_client::lan::playerdata::directory(&self.world_root);
                            self.native_player_save_to_file(uuid, &name, &directory);
                        }
                    }
                    Err(error) => {
                        log::warn!("Sunrise: the owner's playerdata is not saved: {error}")
                    }
                }
            }
        }
        if self.options.execution_role.is_logical_server()
            && let Some(host) = self.lan.as_mut()
        {
            let owner = host.owner.as_ref().map(|owner| owner.uuid);
            // The vehicle tree goes into a player's file when they leave
            // (`take_parent_vehicle`); here it stays in the chunk saves.
            host.hub.guests.update_all(|g| {
                if g.placed
                    && g.participates_in_player_simulation()
                    && !g.released
                    && Some(g.uuid) != owner
                    && !self.crash_save.excludes(g.conn)
                {
                    // Read-only fixture input, correlated with the real save receipt.
                    // Keep this identical on the untouched/fixed selector builds.
                    if std::env::var_os("SUNRISE_DEV_ADVANCEMENT_SELECTOR_AUDIT").is_some() {
                        eprintln!(
                            "advancement selector model raw: {}",
                            serde_json::json!({"conn":g.conn,"name":g.name,
                                "uuid":sunrise_protocol::crypto::uuid_string(g.uuid),
                                "game_time":self.world.game_time,
                                "progress":g.advancements.to_json(&self.adv.tree, sunrise_world::DATA_VERSION)})
                        );
                    }
                    self.player_autosaves.enqueue(g, &self.world_root, self.res.items());
                }
            });
            let host = self
                .lan
                .as_ref()
                .expect("the player list is still installed");
            self.save_connected_guest_statistics(host);
            for guest in host.hub.guests.iter() {
                if Some(guest.uuid) != owner && host.play.contains_key(&guest.conn) {
                    self.save_guest_advancements(host, guest);
                }
            }
        }
        // PlayerDataStorage logs a failed player write and still saves the levels.
        let _ = self.finish_player_autosaves(true);
        owner_saved
    }

    fn save_world_metadata(&mut self, owner_saved: bool) -> Result<()> {
        self.save_maps();
        self.save_random_sequences();
        self.save_fight();
        self.save_wandering_trader();
        self.save_portal_arrivals();
        self.save_scoreboard();
        self.save_bossbars();
        self.save_schedule();
        self.save_warden_tracker();
        self.sync_world_level_data();
        if self.options.execution_role.writes_singleplayer_player() && !owner_saved {
            // No placed owner (including IntegratedServer.initServer), or the stop path
            // already saved them. Name a known profile on creation without writing its file.
            self.level.write_keeping_player(&self.world_root)?;
        } else {
            self.level.write_without_player(&self.world_root)?;
        }
        self.save_world_state();
        Ok(())
    }

    /// Admit a world save cycle and tell mods it is starting, before players,
    /// levels or metadata are written. Rejects re-entry while the notice runs.
    #[inline(always)]
    pub(super) fn begin_world_save(&mut self, wait: bool) -> Result<()> {
        self.admit_level_save()?;
        if !mod_api::__private::enabled_listened(mod_api::HookId::WorldSaveStart) {
            return Ok(());
        }
        struct Guard<'a, 'g>(&'a mut Game<'g>);
        impl Drop for Guard<'_, '_> {
            fn drop(&mut self) {
                self.0.world_save_starting = false;
            }
        }
        self.world_save_starting = true;
        let guard = Guard(self);
        // Mods write their own state into this cycle before any of it (players first) goes to disk.
        guard.0.native_world_save_start(wait);
        Ok(())
    }

    /// The autosave doesn't `wait` for the chunks to be on disk: every
    /// generated chunk is saved, hundreds at a time, encoded and written on
    /// the region writer meanwhile.
    pub(super) fn save_game_with(&mut self, wait: bool) -> Result<()> {
        if self.remote.is_some() {
            return Ok(());
        }
        self.begin_world_save(wait)?;
        let owner_saved = self.save_placed_players();
        self.write_world(wait, owner_saved)
    }

    /// Startup has no placed players; at shutdown PlayerList has already
    /// finished its writes. Do not serialize the integrated server's shell.
    pub(super) fn save_world_after_players(&mut self, owner_saved: bool) -> Result<()> {
        self.admit_level_save()?;
        self.write_world(true, owner_saved)
    }

    fn write_world(&mut self, wait: bool, owner_saved: bool) -> Result<()> {
        // `DedicatedServer.saveAllChunks` tells the management protocol's clients when a save starts and when
        // it ends without failing, for every save: the command, the autosave, the pause.
        let dedicated = self.is_dedicated_server();
        if dedicated {
            dedicated::events::record_server("saving", serde_json::json!({}));
        }
<<<<<<< HEAD
        if wait {
            self.save_level(true)?;
            self.save_parked_levels(true)?;
            self.save_game_metadata()?;
            let _ = self.finish_player_autosaves(true);
            sunrise_client::save::later::flush();
        } else {
            // Starling, the autosave: its small files go to the background writer too, so the
            // frame does not wait on storage.
            sunrise_client::save::later::deferred(|| {
                self.save_level(false)?;
                self.save_parked_levels(false)?;
                self.save_game_metadata()?;
                let _ = self.finish_player_autosaves(false);
                Result::<()>::Ok(())
            })?;
        }
||||||| 46c00106def
        self.save_level(wait)?;
        self.save_parked_levels(wait)?;
        self.save_game_metadata()?;
        let _ = self.finish_player_autosaves(wait);
=======
        self.save_all_levels(wait)?;
        self.save_world_metadata(owner_saved)?;
>>>>>>> ed6772e2118
        if wait {
            self.message = Some((
                sunrise_client::lang::get("commands.save.success"),
                Instant::now(),
            ));
        }
        self.native_world_saved(wait);
        if dedicated {
            dedicated::events::record_server("saved", serde_json::json!({}));
        }
        Ok(())
    }
}

impl Game<'_> {
    pub(super) fn admit_level_save(&self) -> Result<()> {
        if self.world_save_starting {
            return Err("a world save is already starting".into());
        }
        // A carrier transfer that broke after `prepared` stopped the
        // session (`carrier_stop_session`): its recovery replays the
        // affected regions from the disk state the transfer itself
        // wrote, so no later save — the autosave a later tick would
        // run, or any exit path's — may write over them.
        if self.world.carrier_fences.is_broken() {
            return Err(
                "a carrier transfer did not finish; the world is left for recovery at reopen"
                    .into(),
            );
        }
        Self::admit_entity_save()?;
        Ok(())
    }
}

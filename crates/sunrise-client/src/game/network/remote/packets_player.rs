use super::*;

impl Game<'_> {
    /// `C::Login`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_login_packet(&mut self, p: play::Login) {
        // A client's parts, from the tags it plays with.
        if let Err(e) = self.resolve_parts() {
            log::warn!("Sunrise: {e}");
        }
        // Configuration's tags, over the block ids the parts just bound.
        self.install_remote_tags();
        self.refresh_render_visibility();
        self.test_highlights.clear();
        // handleLogin: `debugSubscriber.clear()`.
        self.debug_charts.clear_subscriptions();
        self.forget_synced_recipes();
        self.level.hardcore = p.hardcore;
        // handleLogin: startWaitingForNewLevel with Reason.OTHER, which
        // replaces ServerReconfigScreen.
        if let Some(r) = self.remote.as_mut() {
            r.reconfiguring = None;
            r.clear_attachments();
            r.viewed.clear();
            // A new ClientPacketListener owns a fresh clock manager.
            r.clocks.clear();
            r.clock_game_time = 0;
            self.world.game_time = 0;
        }
        // handleLogin makes a new MultiPlayerGameMode (carried index 0) and a
        // new LocalPlayer (slot 0, no effects, riding nothing: the server
        // pairs a vehicle again with ClientboundSetPassengersPacket).
        self.selected = 0;
        self.potions.effects.clear();
        self.riding = None;
        if let Some(player) = self.player.as_mut() {
            player.entity_id = p.player_id;
        }
        self.native_net
            .holds
            .borrow_mut()
            .clear_cauldron_continuations();
        if let Some(r) = self.remote.as_mut() {
            r.carried.set(Some(0));
            r.pending_cauldrons.get_mut().clear();
            // handleLogin makes a new ClientLevel: its predictions count from 0.
            r.sequence = 0;
            r.own_invisible = false;
            r.own_on_fire = false;
            r.glowing.remove(&p.player_id);
            r.online_mode = p.online_mode;
            r.levels.clone_from(&p.levels);
            r.enforces_secure_chat = p.enforces_secure_chat;
            r.entity_id = p.player_id;
            r.local_entity_silent = false;
            // handleLogin: the new LocalPlayer is the camera.
            r.camera = None;
            r.logged_in = true;
            r.view_distance = p.chunk_radius;
            r.simulation_distance = p.simulation_distance;
            r.show_death_screen = p.show_death_screen;
            r.reduced_debug_info = p.reduced_debug_info;
            r.do_limited_crafting = p.limited_crafting;
        }
        self.bind_local_player_residency();
        self.refresh_debug_renderers();
        // LocalPlayer.setReducedDebugInfo always rebuilds, also on a second login.
        self.debug_entries.invalidate();
        self.streaming.radius = self
            .settings
            .render_distance
            .min(p.chunk_radius.saturating_add(1))
            .max(2);

        // handleLogin: startWaitingForNewLevel(.., Reason.OTHER).
        self.enter_remote_dimension(
            &p.spawn,
            RemoteDimensionEntry::Login,
            sunrise_client::level_loading::Reason::Other,
        );
        // No ClientInformation here: the configuration phase sent it,
        // and ClientPacketListener.broadcastClientInformation sends it
        // again only when an option changes it.
        self.remote_chat_login();
        if let Some(remote) = &self.remote {
            self.native_client_player_logging_in(p.player_id, remote.profile);
        }
    }

    /// `C::Respawn`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_respawn_packet(&mut self, p: play::Respawn) {
        self.reset_client_item_use();
        // The new LocalPlayer copies reduced debug info through its setter.
        self.debug_entries.invalidate();
        let old_player = self.remote.as_ref().map(|r| (r.entity_id, r.profile));
        if let Some((_, uuid)) = old_player {
            self.forget_entity_initialization(Holder::Player(uuid));
        }
        if let Some(r) = self.remote.as_ref() {
            self.audio.remove_sound_entity(r.entity_id as u32);
        }
        // A new player, without effects (the server sends them
        // again) and with no item in use.
        self.potions.effects.clear();
        // Every synced attachment but the player's is sent again; the viewed
        // levels go or stay with the level entered (`enter_respawn_level`).
        if let Some(r) = self.remote.as_mut() {
            r.attachments
                .retain(|holder, _| matches!(holder, Holder::Player(_)));
        }
        self.using = None;
        // After a death (no KEEP_ENTITY_DATA) the new LocalPlayer
        // rides nothing: the server pairs a vehicle again with its
        // passengers. A dimension change keeps the vehicle it travels in.
        // handleRespawn: KEEP_ENTITY_DATA gives the new player the old
        // one's motion, `resetPos` none.
        let kept_velocity = if p.keep & 2 != 0 {
            self.player.as_ref().map(|pl| pl.velocity)
        } else {
            None
        };
        if let Some(r) = self.remote.as_mut() {
            r.respawn_velocity = kept_velocity;
        }
        if p.keep & 2 == 0 {
            self.reset_remote_player_riding();
            // KEEP_ENTITY_DATA copies the old player's synched data
            // (its shared flags) to the new one; without it they reset.
            if let Some(r) = self.remote.as_mut() {
                r.own_invisible = false;
                r.own_on_fire = false;
                r.glowing.remove(&r.entity_id);
                r.local_entity_silent = false;
            }
        }
        // handleRespawn compares the levels' keys: a mod's level is
        // its own dimension even where `Dim` has no name for it.
        let dimension_changed = self
            .remote
            .as_ref()
            .is_some_and(|r| r.level != p.spawn.dimension);
        let died = self.screen == Screen::Dead
            || self
                .player
                .as_ref()
                .is_some_and(|m| m.health.health() <= 0.0);
        let reason = sunrise_client::level_loading::reason_for_respawn(
            died,
            &p.spawn.dimension,
            self.dim.id(),
        );
        if dimension_changed {
            sunrise_client::spans::startup::arm("dimension.change");
        }
        // handleRespawn makes a new ClientLevel for another dimension
        // key (a proxy's server switch too): its predictions count from 0.
        self.native_net
            .holds
            .borrow_mut()
            .clear_cauldron_continuations();
        if let Some(r) = self.remote.as_mut() {
            r.pending_cauldrons.get_mut().clear();
            if r.level != p.spawn.dimension {
                r.sequence = 0;
            }
        }
        self.portals.respawn_attributes.respawn(p.keep & 1 != 0);
        let entry = if died {
            RemoteDimensionEntry::DeathRespawn
        } else {
            RemoteDimensionEntry::Respawn
        };
        self.enter_respawn_level(&p, entry, reason);
        // handleRespawn adds the new LocalPlayer after what the kept
        // level holds; a new level holds nothing yet.
        let before: std::collections::HashSet<u32> = self
            .mobs
            .iter()
            .map(|m| m.id)
            .chain(self.boats.iter().map(|b| b.id))
            .collect();
        if let Some(r) = self.remote.as_mut() {
            r.ticked_before_player = before;
        }
        if dimension_changed {
            // LevelExtractor.setLevel: a new level drops the highlights.
            self.test_highlights.clear();
            let (music, _) = self.situational_music();
            self.music.stop_playing(music.as_ref(), &mut self.audio);
        }
        self.set_screen(Screen::Playing);
        if let Some(r) = self.remote.as_mut() {
            r.death_message = None;
            r.health = (20.0, 20, 5.0);
            // handleRespawn: the new LocalPlayer is the camera.
            r.camera = None;
        }
        self.capture(true);
        if let Some((old_id, uuid)) = old_player {
            self.native_client_player_clone(
                old_id,
                self.own_entity_id(),
                uuid,
                died,
                dimension_changed,
            );
        }
    }

    /// `C::SetHealth`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_health_packet(&mut self, p: play::SetHealth) {
        // ClientPacketListener.handleSetHealth: LocalPlayer.hurtTo.
        let flash = self.remote.as_mut().is_some_and(|r| {
            r.health = (p.health, p.food, p.saturation);
            std::mem::replace(&mut r.flash_on_set_health, true)
        });
        if flash && let Some(pl) = self.player.as_mut() {
            pl.health.flash_health_change(p.health);
        }
        self.apply_remote_health();
        if p.health <= 0.0 {
            self.reset_client_item_use();
        }
        // The death screen opens (or the respawn is asked for) with the
        // death packet when the rule hides it.
        let shown = self.remote.as_ref().is_none_or(|r| r.show_death_screen);
        if p.health <= 0.0 && shown && self.screen != Screen::Dead && self.player.is_some() {
            self.close_dialogs();
            self.set_screen(Screen::Dead);
            self.ui.death_time = 0;
            self.ui.death_confirm = None;
            self.capture(false);
        }
    }

    /// `C::SetExperience`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_experience_packet(&mut self, p: play::SetExperience) {
        self.experience = Experience {
            level: p.level,
            progress: p.progress,
            total: p.total,
        }
    }

    /// `C::PlayerAbilities`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_player_abilities_packet(&mut self, p: play::PlayerAbilities) {
        self.creative = p.flags & 8 != 0;
        if let Some(r) = self.remote.as_mut() {
            r.abilities = Some(p);
        }
        self.apply_remote_abilities();
    }

    /// `ClientPacketListener.handleContainerContent` initializes only the addressed menu.
    pub(super) fn apply_container_set_content_packet(&mut self, p: play::ContainerSetContent) {
        let open = self
            .remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0);
        if p.container == 0 || p.container == open {
            self.remote_menu_state(p.container, p.state);
            for (slot, item) in p.items.iter().enumerate() {
                let stack = self.to_stack(item);
                self.set_menu_slot(p.container, slot as i32, stack);
            }
            let carried = self.to_stack(&p.carried);
            if p.container == open {
                self.carried = carried;
            } else if let Some(r) = self.remote.as_mut() {
                r.inventory_carried = carried;
            }
            self.remote_synced();
        }
    }

    /// `C::ContainerSetSlot`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_container_set_slot_packet(&mut self, p: play::ContainerSetSlot) {
        self.remote_menu_state(p.container, p.state);
        let stack = self.to_stack(&p.item);
        self.tutorial_get_item(stack.as_ref());
        // ClientPacketListener.handleContainerSetSlot: a hotbar slot that
        // gains items squeezes for 5 ticks.
        if p.container == 0
            && (36..=45).contains(&p.slot)
            && let Some(new) = &stack
        {
            let (pop, old) = match p.slot {
                45 => (9, self.offhand.as_ref()),
                s => (s as usize - 36, self.inventory[s as usize - 36].as_ref()),
            };
            if old.is_none_or(|old| old.count < new.count) {
                self.hotbar_pop[pop] = 5;
            }
        }
        let before = self.creative_stacks();
        self.set_menu_slot(p.container, i32::from(p.slot), stack);
        self.remote_synced_since(&before);
    }

    /// `C::SetCursorItem`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_cursor_item_packet(&mut self, p: play::SetCursorItem) {
        let stack = self.to_stack(&p.item);
        self.tutorial_get_item(stack.as_ref());
        self.carried = stack;
    }

    /// `C::SetPlayerInventory`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_player_inventory_packet(&mut self, p: play::SetPlayerInventory) {
        let stack = self.to_stack(&p.item);
        self.tutorial_get_item(stack.as_ref());
        let before = self.creative_stacks();
        let held = match p.slot {
            0..=35 => self.inventory[p.slot as usize].as_ref(),
            40 => self.offhand.as_ref(),
            _ => None,
        };
        let changed = !super::super::hand::matches_for_swap(held, stack.as_ref());
        if let Ok(slot) = usize::try_from(p.slot) {
            self.remote_hand_slot_set(slot, changed);
        }
        match p.slot {
            0..=35 => self.inventory[p.slot as usize] = stack,
            36..=39 => self.armor[3 - (p.slot - 36) as usize] = stack,
            // Inventory.SLOT_OFFHAND.
            40 => self.offhand = stack,
            _ => {}
        }
        self.remote_synced_since(&before);
    }

    /// `C::SetHeldSlot`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_held_slot_packet(&mut self, p: play::SetHeldSlot) {
        // ClientPacketListener.handleSetHeldSlot: a hotbar slot only, and the
        // carried index stays, so the next tick answers with SetCarriedItem.
        if (0..9).contains(&p.slot) {
            self.selected = p.slot as usize;
        }
    }

    /// `C::PlayerInfoUpdate`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_player_info_update_packet(&mut self, p: play::PlayerInfoUpdate) {
        if let Some(r) = self.remote.as_mut() {
            // handlePlayerInfoUpdate: new entries first, then every
            // action on every known entry.
            // INITIALIZE_CHAT: the session (or none) each entry carries; the
            // player's validator is made again when their chat is next read.
            if p.actions & 2 != 0 {
                for e in &p.entries {
                    r.chat_sessions.insert(e.uuid, e.chat_session.clone());
                    r.chat_validators.remove(&e.uuid);
                }
            }
            if p.actions & 1 != 0 {
                for e in &p.entries {
                    if let Some(name) = &e.name {
                        r.tab_players.entry(e.uuid).or_insert_with(|| {
                            sunrise_client::tab_list::TabPlayer::new(e.uuid, name.clone())
                        });
                        if let Some(t) = e.properties.iter().find(|p| p.name == "textures") {
                            // Without the services' profile property keys a
                            // signature can't be checked; a signed property
                            // counts as genuine.
                            use sunrise_client::skins::{self, Signature};
                            let signature = if t.signature.is_some() {
                                Signature::Signed
                            } else {
                                Signature::Unsigned
                            };
                            let textures =
                                skins::unpack(&t.value, signature, skins::texture_uris());
                            r.skin_textures.entry(e.uuid).or_insert(textures);
                        }
                    }
                }
            }
        }
        for e in p.entries {
            if mod_api::__private::listened(mod_api::HookId::ClientPlayerChangeGameType)
                && self.player.is_some()
                && let Some(mode) = e.game_mode
                && let Some(r) = self.remote.as_ref()
                && r.profile == e.uuid
                && let Some(info) = r.tab_players.get(&e.uuid)
                && info.game_mode != mode
            {
                let current = info.game_mode;
                self.native_client_player_change_game_type(
                    self.own_entity_id(),
                    e.uuid,
                    current,
                    mode,
                );
            }
            if let Some(r) = self.remote.as_mut() {
                if let Some(t) = r.tab_players.get_mut(&e.uuid) {
                    if let Some(mode) = e.game_mode {
                        t.game_mode = mode;
                    }
                    if let Some(latency) = e.latency {
                        t.latency = latency;
                    }
                    if let Some(name) = &e.display_name {
                        t.display_name = name.as_ref().map(sunrise_client::scoreboard::tag_to_json);
                    }
                    if let Some(order) = e.list_order {
                        t.list_order = order;
                    }
                    if let Some(hat) = e.show_hat {
                        t.show_hat = hat;
                    }
                    match e.listed {
                        Some(true) => {
                            r.listed.insert(e.uuid);
                        }
                        Some(false) => {
                            r.listed.remove(&e.uuid);
                        }
                        None => {}
                    }
                }
                if let Some(name) = e.name {
                    r.players.insert(e.uuid, name);
                }
            }
        }
    }

    /// `C::PlayerInfoRemove`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_player_info_remove_packet(&mut self, p: play::PlayerInfoRemove) {
        if let Some(r) = self.remote.as_mut() {
            for u in p.uuids {
                r.players.remove(&u);
                r.tab_players.remove(&u);
                r.chat_sessions.remove(&u);
                r.chat_validators.remove(&u);
                r.skin_textures.remove(&u);
                r.listed.remove(&u);
                r.attachments.remove(&Holder::Player(u));
            }
        }
    }

    /// `C::TabList`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_tab_list_packet(&mut self, p: play::TabList) {
        // handleTabListCustomisation: empty text clears it.
        let text = |t: &Tag| {
            let json = sunrise_client::scoreboard::tag_to_json(t);
            let text = sunrise_client::scoreboard::comp_text(&json);
            text.spans
                .iter()
                .any(|span| !span.text.is_empty())
                .then_some(text)
        };
        self.tab_list.header = text(&p.header);
        self.tab_list.footer = text(&p.footer);
    }

    /// `C::Commands`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_commands_packet(&mut self, p: play::Commands) {
        let tree = sunrise_client::command::network::from_packet(
            &p,
            sunrise_client::command::vanilla::dispatcher(),
        );
        // The chat's tree follows the server's now; a mod's commands
        // go into it again (`chat_tree_for`).
        let chat_tree = self.chat_tree_for(Some(&tree));
        if let Some(r) = self.remote.as_mut() {
            r.chat_tree = chat_tree;
            r.chat_tree_received = true;
        }
    }

    /// `C::DisguisedChat`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_disguised_chat_packet(&mut self, p: play::DisguisedChat) {
        let content = sunrise_client::scoreboard::tag_to_json(&p.message);
        let narration = self
            .decorate_chat(&p.chat_type, &content, Decoration::Narration)
            .plain();
        let line = self.decorate_chat(&p.chat_type, &content, Decoration::Chat);
        self.handle_player_chat(QueuedChat {
            line,
            signature: None,
            trust: ChatTrust::Secure,
            narration,
            sender: 0,
            chat_type: p.chat_type.chat_type,
            player: None,
        });
    }

    /// `C::SetDisplayObjective`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_display_objective_packet(&mut self, p: play::SetDisplayObjective) {
        // DisplaySlot ids: list, sidebar, below_name, then the team sidebars.
        let slot = match p.slot {
            0 => "list".to_owned(),
            2 => "below_name".to_owned(),
            n @ 3..=18 => format!(
                "sidebar.team.{}",
                sunrise_client::text::COLORS[(n - 3) as usize].1
            ),
            _ => "sidebar".to_owned(),
        };
        let board = &mut self.score.board;
        let objective = board.objective(&p.objective).map(|o| o.name.clone());
        board.set_display_objective(&slot, objective.as_deref());
    }

    /// `C::SetScore`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_score_packet(&mut self, p: play::SetScore) {
        let board = &mut self.score.board;
        if board.objective(&p.objective).is_some() {
            let s = board.score_mut(&p.owner, &p.objective);
            s.value = p.score;
            s.display = p
                .display
                .as_ref()
                .map(sunrise_client::scoreboard::tag_to_json);
            s.number_format = p.number_format.as_ref().map(number_format);
        }
    }

    /// `C::ClearTitles`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_clear_titles_packet(&mut self, p: play::Bool1) {
        self.titles.clear();
        if p.value {
            self.titles.reset_times();
        }
    }

    /// `C::PlayerCombatKill`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_player_combat_kill_packet(
        &mut self,
        p: play::PlayerCombatKill,
    ) -> Result<()> {
        // The component resolves in the client's current language, as
        // the chat line does, not in the language the session joined
        // with (`Remote.lang` is the vanilla server-language table).
        let message = remote_text(&p.message);
        let me = self.remote.as_ref().is_some_and(|r| r.entity_id == p.id);
        if let Some(r) = self.remote.as_mut() {
            r.death_message = Some(message);
        }
        if me {
            self.reset_client_item_use();
        }
        // ClientPacketListener.handlePlayerCombatKill: the death
        // screen, whatever health was last sent, or `respawn()` at
        // once when it is hidden.
        if me && self.screen != Screen::Dead && self.player.is_some() {
            if self.remote.as_ref().is_none_or(|r| r.show_death_screen) {
                self.close_dialogs();
                self.set_screen(Screen::Dead);
                self.ui.death_time = 0;
                self.ui.death_confirm = None;
                self.capture(false);
            } else {
                self.respawn_player()?;
            }
        }
        Ok(())
    }

    /// `C::SystemChat`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_system_chat_packet(&mut self, p: play::SystemChat) -> Result<()> {
        // The component as sent, translated in the client's own
        // language; mods may change or cancel it. ChatListener.handleOverlay
        // for the action bar.
        let mut text = remote_text(&p.content);
        if !self.chat_received(true, p.overlay, 0, None, None, &mut text) {
            return Ok(());
        }
        if p.overlay {
            self.say_system_queued(&text);
            self.titles.set_overlay(text, false);
        } else if self.add_chat_line(text.clone(), sunrise_client::chat::ChatSource::Server) {
            // handleSystemMessage: said once the chat takes it.
            self.say_system_chat_queued(&text);
        }
        Ok(())
    }

    /// `C::PlayerChat`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_player_chat_packet(&mut self, p: play::PlayerChat) -> Result<()> {
        use sunrise_client::scoreboard::tag_to_json;
        // SignedMessageBody.Packed.unpack: the last seen it names must
        // be in the cache; then the message joins it
        // (MessageSignatureCache.push).
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        // handlePlayerChat: the global index counts every chat packet; one out of
        // order disconnects.
        let expected = r.next_chat_index;
        r.next_chat_index = r.next_chat_index.wrapping_add(1);
        if p.global_index != expected {
            log::error!(
                "Missing or out-of-order chat message from server, expected index {expected} but got {}",
                p.global_index
            );
            let reason = Text::translate("multiplayer.disconnect.bad_chat_index", &[]);
            return self.leave_server(Some(reason));
        }
        let last_seen: Option<Vec<Signature>> =
            p.last_seen.iter().map(|s| r.signatures.unpack(s)).collect();
        let Some(last_seen) = last_seen else {
            log::error!(
                "Message from player with ID {:032x} referenced unrecognized signature id",
                p.sender
            );
            let reason = Text::translate("multiplayer.disconnect.invalid_packet", &[]);
            return self.leave_server(Some(reason));
        };
        let signature = p
            .signature
            .as_deref()
            .and_then(sunrise_protocol::secure_chat::signature);
        r.signatures.push(&last_seen, signature.as_ref());
        // The sender's PlayerInfo: none is an error line; else their
        // `SignedMessageValidator` decides (`updateAndValidate`), and a message it
        // takes without a signature of its own shows as unsigned.
        if !r.tab_players.contains_key(&p.sender) {
            log::error!(
                "Received player chat packet for unknown player with ID: {:032x}",
                p.sender
            );
            self.handle_chat_message_error(&p.chat_type, signature.as_ref());
            return Ok(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let incoming = secure_chat::Incoming {
            sender: p.sender,
            index: p.index,
            signature: signature.as_ref(),
            body: secure_chat::Body {
                content: &p.content,
                timestamp: p.timestamp,
                salt: p.salt,
                last_seen: &last_seen,
            },
            unsigned_content: p.unsigned_content.as_ref(),
            filter: &p.filter,
        };
        let Some(validated) = r.validate_chat(&incoming, now) else {
            self.handle_chat_message_error(&p.chat_type, signature.as_ref());
            return Ok(());
        };
        let signature = signature.filter(|_| validated == secure_chat::Validated::Signed);
        // ChatListener.handlePlayerChatMessage: Only Show Secure Chat
        // shows the signed content, never the server's replacement.
        // The narrator says the message's own decorated content
        // (`narrateChatMessage(boundChatType, message.decoratedContent())`).
        let decorated = p
            .unsigned_content
            .as_ref()
            .map_or_else(|| serde_json::Value::from(p.content.as_str()), tag_to_json);
        let content = if self.settings.file.bool("onlyShowSecureChat") {
            serde_json::Value::from(p.content.as_str())
        } else {
            decorated.clone()
        };
        let narration = self
            .decorate_chat(&p.chat_type, &decorated, Decoration::Narration)
            .plain();
        let line = self.decorate_chat(&p.chat_type, &content, Decoration::Chat);
        let trust = self.chat_trust(
            p.sender,
            signature.is_some(),
            p.timestamp,
            &p.content,
            p.unsigned_content.as_ref(),
            &line,
        );
        // A hidden or fully filtered message counts as processed, not shown.
        let Some((line, narration)) = self.shown_player_chat(&p, line, narration) else {
            if let Some(signature) = &signature {
                self.mark_message_processed(signature, false);
            }
            return Ok(());
        };
        let listened = mod_api::__private::listened(mod_api::HookId::ClientChatReceived);
        let player = listened.then(|| chat_received::player_chat_info(&p, signature.is_some()));
        self.handle_player_chat(QueuedChat {
            line,
            signature,
            trust,
            narration,
            sender: p.sender,
            chat_type: p.chat_type.chat_type,
            player,
        });
        Ok(())
    }

    /// `C::AwardStats`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_award_stats_packet(&mut self, p: play::AwardStats) -> Result<()> {
        // handleAwardStats: StatsCounter.setValue for each.
        let Some(r) = self.remote.as_ref() else {
            return Ok(());
        };
        let mut known = Vec::new();
        for (ty, value, count) in p.stats {
            let Some(ty) = r
                .builtin
                .get("minecraft:stat_type")
                .and_then(|t| t.name(ty))
            else {
                continue;
            };
            let values = match ty {
                "minecraft:mined" => "minecraft:block",
                "minecraft:killed" | "minecraft:killed_by" => "minecraft:entity_type",
                "minecraft:custom" => "minecraft:custom_stat",
                _ => "minecraft:item",
            };
            if let Some(key) = r.builtin.get(values).and_then(|v| v.name(value)) {
                known.push((ty.to_owned(), key.to_owned(), count));
            }
        }
        for (ty, key, count) in known {
            self.stats.values.entry(ty).or_default().insert(key, count);
        }
        Ok(())
    }

    /// `C::ContainerSetData`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_container_set_data_packet(
        &mut self,
        p: play::ContainerSetData,
    ) -> Result<()> {
        if self.open_menu_name(p.container) == "minecraft:lectern" {
            self.remote_lectern_page(i32::from(p.value));
            return Ok(());
        }
        if self.remote_station_open(p.container) {
            self.remote_station_data(usize::try_from(p.id).unwrap_or(usize::MAX), p.value);
            return Ok(());
        }
        if usize::try_from(p.id)
            .is_ok_and(|index| self.native_screen_data(p.container, index, i32::from(p.value)))
        {
            return Ok(());
        }
        let v = i32::from(p.value);
        match self
            .container
            .and_then(|key| self.world.block_entities.get_mut(&key))
        {
            Some(BlockEntity::Furnace(f)) => match p.id {
                0 => f.lit_remaining = v,
                1 => f.lit_total = v,
                2 => f.cooking_timer = v,
                3 => f.cooking_total = v,
                _ => {}
            },
            // CrafterBlockEntity.containerData.
            Some(BlockEntity::Crafter(c)) => match usize::try_from(p.id) {
                Ok(k @ 0..9) => c.disabled[k] = v == 1,
                Ok(9) => c.triggered = v,
                _ => {}
            },
            // BrewingStandBlockEntity.dataAccess.
            Some(BlockEntity::Brewing(b)) => match p.id {
                0 => b.brew_time = v,
                1 => b.fuel = v,
                2 => b.total_brew_time = v,
                3 => b.total_fuel = v,
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }
}

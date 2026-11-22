use super::*;

impl Remote {
    /// The interpolation a boat or other object `id` had toward the server's
    /// position, taken out: its target, rotation and steps to go.
    pub(in crate::game) fn take_object_lerp(
        &mut self,
        id: u32,
    ) -> Option<([f64; 3], Option<(f32, f32)>, u32)> {
        self.lerps
            .remove(&(id as i32))
            .map(|l| (l.target, l.rotation, l.steps))
    }
    pub(in crate::game) fn set_menu_title(&mut self, title: Option<sunrise_client::text::Text>) {
        self.menu_title_snapshot = title.try_into_api().ok().flatten().map(Arc::new);
        self.menu_title = title;
    }

    pub(in crate::game) fn menu_title_snapshot(&self) -> Option<&mod_api::Text> {
        self.menu_title_snapshot.as_deref()
    }

    pub(in crate::game) fn shared_menu_title_snapshot(&self) -> Option<Arc<mod_api::Text>> {
        self.menu_title_snapshot.clone()
    }

    /// Resolve registry keys only when registries or the dimension change.
    pub(super) fn resolve_clocks(&mut self) {
        self.overworld_clock = self
            .registries
            .id("minecraft:world_clock", "minecraft:overworld");
        self.default_clock = self
            .dimension
            .default_clock
            .as_deref()
            .and_then(|name| self.registries.id("minecraft:world_clock", name));
        for id in [self.overworld_clock, self.default_clock]
            .into_iter()
            .flatten()
        {
            self.clocks.entry(id).or_insert(play::ClockState {
                clock: id,
                total_ticks: 0,
                partial_tick: 0.0,
                rate: 1.0,
            });
        }
    }

    pub(super) fn clock_time(&self, id: Option<i32>) -> i64 {
        id.and_then(|id| self.clocks.get(&id))
            .map_or(0, |c| c.total_ticks)
    }

    pub(in crate::game) fn overworld_clock_time(&self) -> i64 {
        self.clock_time(self.overworld_clock)
    }

    /// `Minecraft.level != null`: from `handleLogin` until the next
    /// `handleConfigurationStart` (`clearClientLevel`). Minecraft.tick sends
    /// CLIENT_TICK_END only then, so a client waiting for the server's Login
    /// (a busy server takes seconds to place a new player) sends nothing.
    pub(in crate::game) fn has_level(&self) -> bool {
        client_has_level(self.logged_in, self.reconfiguring)
    }

    /// An entity the server added to this client level: its type
    /// (`minecraft:text_display`) and its position as last sent.
    pub(in crate::game) fn tracked_entity(&self, id: i32) -> Option<(&str, [f64; 3])> {
        if let Some(row) = self.general.rows.get(&id) {
            return Some((row.kind(), row.position));
        }
        self.entities
            .get(&id)
            .map(|t| (t.kind.as_str(), t.position))
    }

    /// The profile of the player entity `id`: this window's own player, or
    /// another one by the UUID it was added with and its profile name.
    pub(in crate::game) fn level_player(&self, id: i32) -> Option<(u128, Option<&str>)> {
        if id == self.entity_id {
            return Some((self.profile, Some(self.profile_name.as_str())));
        }
        let uuid = *self.level_players.get(&id)?;
        Some((uuid, self.level_player_names.get(&id).map(String::as_str)))
    }

    /// `PlayerInfo.getMessageValidator().updateAndValidate(message)` for `message.sender`.
    pub(super) fn validate_chat(
        &mut self,
        message: &secure_chat::Incoming,
        now: i64,
    ) -> Option<secure_chat::Validated> {
        let sender = message.sender;
        if let Some(validator) = self.chat_validators.get_mut(&sender) {
            return validator.update_and_validate(message, now);
        }
        let (mut validator, settled) = self.new_chat_validator(sender);
        let validated = validator.update_and_validate(message, now);
        // While the services' keys are still on their way the validator is a stopgap, not kept.
        if settled {
            self.chat_validators.insert(sender, validator);
        }
        validated
    }

    /// `ClientPacketListener.initializeChatSession` for `sender`'s entry: a session the services'
    /// keys vouch for makes a keyed validator, anything else the fallback of
    /// `enforcesSecureChat()` (`canValidateProfileKeys() && serverEnforcesSecureChat`). `false`
    /// while the keys are not known yet.
    pub(super) fn new_chat_validator(&self, sender: u128) -> (secure_chat::MessageValidator, bool) {
        let keys = self.services_keys.lock().unwrap_or_else(|e| e.into_inner());
        let (can_validate, settled) = match &*keys {
            ServicesKeys::Ready(_) => (true, true),
            ServicesKeys::Missing => (false, true),
            ServicesKeys::Pending => (false, false),
        };
        let fallback =
            secure_chat::MessageValidator::fallback(can_validate && self.enforces_secure_chat);
        let (ServicesKeys::Ready(keys), Some(Some((id, expires_at, key, signature)))) =
            (&*keys, self.chat_sessions.get(&sender))
        else {
            if matches!(&*keys, ServicesKeys::Missing)
                && self.chat_sessions.get(&sender).is_some_and(Option::is_some)
            {
                log::warn!(
                    "Ignoring chat session from {sender:032x} due to missing Services public key"
                );
            }
            return (fallback, settled);
        };
        let profile_key = secure_chat::ProfileKey {
            expires_at: *expires_at,
            key: key.clone(),
            signature: signature.clone(),
        };
        match secure_chat::ChatSession::validate(*id, profile_key, keys, sender) {
            Ok(session) => (secure_chat::MessageValidator::keyed(session), true),
            Err(why) => {
                log::error!("Failed to validate profile key for player {sender:032x}: {why}");
                (fallback, true)
            }
        }
    }

    /// How far the server sends chunks: its view distance and the ring
    /// around (`ChunkTrackingView`), at least two.
    pub(in crate::game) fn sent_radius(&self) -> i32 {
        self.view_distance.max(2).saturating_add(1)
    }

    /// A player entity's profile name: the one it was created with, else its
    /// player-list entry's (an entity the list named after it spawned).
    pub(super) fn level_player_name(&self, id: i32, uuid: &u128) -> Option<&str> {
        self.level_player_names
            .get(&id)
            .or_else(|| self.players.get(uuid))
            .map(String::as_str)
    }

    /// Listed UUIDs always have a tab entry; removal clears both together.
    pub(in crate::game) fn listed_player_count(&self) -> usize {
        self.listed.len()
    }

    /// The local player's profile UUID.
    pub(in crate::game) fn profile(&self) -> u128 {
        self.profile
    }

    /// Another player's profile UUID, by their entity id (None for the local player).
    pub(super) fn player_uuid(&self, id: u32) -> Option<u128> {
        let uuid = *self.level_players.get(&(id as i32))?;
        (uuid != self.profile).then_some(uuid)
    }

    /// The entity id of a player in the level, by profile UUID.
    pub(in crate::game) fn player_entity(&self, uuid: u128) -> Option<u32> {
        self.level_players
            .iter()
            .find(|(_, u)| **u == uuid)
            .map(|(id, _)| *id as u32)
    }

    /// The client's synced Entity shared flag 0, including the local player.
    pub(in crate::game) fn entity_on_fire(&self, id: i32) -> Option<bool> {
        if id == self.entity_id {
            Some(self.own_on_fire)
        } else {
            self.entities.get(&id).map(|t| t.on_fire)
        }
    }
    pub(in crate::game) fn tracked_fire_immunity(&self, id: i32) -> Option<bool> {
        Some(self.entities.get(&id)?.mod_fire_immune)
    }
    pub(in crate::game) fn set_tracked_fire_immunity(&mut self, id: i32, immune: bool) -> bool {
        self.entities.get_mut(&id).is_some_and(|entity| {
            entity.mod_fire_immune = immune;
            true
        })
    }
    /// The kind and last sent position of a tracked server entity.
    pub(in crate::game) fn tracked(&self, id: i32) -> Option<(&str, [f64; 3])> {
        self.entities.get(&id).map(|t| (&*t.kind, t.position))
    }
}

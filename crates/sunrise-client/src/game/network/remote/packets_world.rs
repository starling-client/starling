use super::*;

impl Game<'_> {
    /// `C::ForgetLevelChunk`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_forget_level_chunk_packet(&mut self, p: play::ForgetLevelChunk) {
        // `poll_remote` forgets a run of them at once.
        self.forget_remote_columns(&mut [(p.pos[0], p.pos[1])].into_iter().collect(), true);
    }

    /// `C::BlockDestruction`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_block_destruction_packet(&mut self, p: play::BlockDestruction) {
        // ClientPacketListener.handleBlockDestruction: another player's crack.
        self.destroy_block_progress(p.id, p.pos, i32::from(p.progress));
    }

    /// `C::SectionBlocksUpdate`: apply verified states in packet order.
    pub(in crate::game) fn apply_section_blocks_update_packet(
        &mut self,
        p: &play::SectionBlocksUpdate,
        mut groups: crate::game::native_connection_groups::GroupUpdates,
    ) {
        for (pos, state) in super::super::remote_chunks::section_blocks(p) {
            self.server_block(pos, state, groups.remove(&pos));
        }
    }

    /// `C::Sound`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_sound_packet(&mut self, p: play::Sound) {
        if let Some(name) = self.remote_sound_name(&p.sound)
            && !self.pvp_mutes_sound(&name)
        {
            let at = [
                f64::from(p.x) / 8.0,
                f64::from(p.y) / 8.0,
                f64::from(p.z) / 8.0,
            ];
            let mut sound = LevelSound::new(&name, at, p.volume, p.pitch);
            if let Ok(source) = usize::try_from(p.source)
                && let Some(source) = mod_api::SoundSource::ALL.get(source)
            {
                sound.source = *source;
            }
            self.play_client_level_sound(sound, p.seed);
        }
    }

    /// `C::SoundEntity`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_sound_entity_packet(&mut self, p: play::SoundEntity) {
        if let Some(name) = self.remote_sound_name(&p.sound)
            && !self.pvp_mutes_sound(&name)
        {
            let mut sound = LevelSound::new(&name, [0.0; 3], p.volume, p.pitch);
            sound.emitter = Some(p.id as u32);
            if let Ok(source) = usize::try_from(p.source)
                && let Some(source) = mod_api::SoundSource::ALL.get(source)
            {
                sound.source = *source;
            }
            self.play_client_level_sound(sound, p.seed);
        }
    }

    /// `C::SetTime`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_time_packet(&mut self, p: play::SetTime) {
        // ClientPacketListener.handleSetTime / ClientClockManager.handleUpdates:
        // advance all clocks to the packet's game time before applying its
        // (possibly partial) updates. The sky still uses only the default clock.
        self.world.game_time = p.game_time;
        self.remote_clock_tick();
        if let Some(r) = self.remote.as_mut() {
            for clock in p.clocks {
                if r.registries
                    .name("minecraft:world_clock", clock.clock)
                    .is_some()
                {
                    r.clocks.insert(clock.clock, clock);
                }
            }
            self.world.day_time = r.clock_time(r.default_clock);
        }
    }

    /// `C::InitializeBorder`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_initialize_border_packet(&mut self, p: play::InitializeBorder) {
        let b = &mut self.border;
        b.set_center([p.center_x, p.center_z]);
        if p.lerp_time > 0 {
            b.lerp_between(p.old_size, p.new_size, p.lerp_time);
        } else {
            b.set_size(p.new_size);
        }
        b.set_absolute_max_size(p.absolute_max_size);
        b.warning_distance = p.warning_blocks;
        b.warning_time = p.warning_time;
    }

    /// `C::SetBorderLerpSize`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_border_lerp_size_packet(&mut self, p: play::SetBorderLerpSize) {
        self.border
            .lerp_between(p.old_size, p.new_size, p.lerp_time);
    }

    /// `C::SetDefaultSpawnPosition`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_set_default_spawn_position_packet(
        &mut self,
        p: play::SetDefaultSpawnPosition,
    ) {
        // handleSetSpawn: ClientLevel.setRespawnData, which spawn compasses point at: the
        // position with the level it is in, which the compass compares with its own.
        let [x, y, z] = p.pos.pos;
        self.spawn = Some(Vec3::new(
            f64::from(x) + 0.5,
            f64::from(y),
            f64::from(z) + 0.5,
        ));
        self.level.world_spawn_dimension =
            (p.pos.dimension != "minecraft:overworld").then_some(p.pos.dimension);
    }

    /// `C::BlockEvent`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_block_event_packet(&mut self, p: play::BlockEvent) -> Result<()> {
        // Level.blockEvent on the client level: the block's triggerEvent
        // runs on the client's copy of whatever is at pos (a piston's
        // moveBlocks builds the moving block entities it animates, a note
        // block plays); what it doesn't handle reaches the client copy of
        // the block entity (chest lids, shulker boxes).
        let pos = sunrise_core::position::BlockPos::new(p.pos[0], p.pos[1], p.pos[2]);
        let triggered = self.with_level(|b, level, ticks| {
            b.client_block_event(level, ticks, pos, i32::from(p.b0), i32::from(p.b1))
        });
        self.moving_pistons_stale = true;
        self.process_level_events()?;
        if !triggered {
            let state = self.world.block(p.pos[0], p.pos[1], p.pos[2]);
            let block = &self.res.facts().get(state).name;
            let named = self.remote.as_ref().and_then(|r| {
                r.builtin
                    .get("minecraft:block")?
                    .name(p.block)
                    .map(str::to_owned)
            });
            if named.as_deref() == Some(&**block) {
                self.block_entity_event(p.pos, i32::from(p.b0), i32::from(p.b1));
            }
        }
        Ok(())
    }

    /// `C::GameEvent`: its arms of `apply_packet`, guard and order kept.
    pub(super) fn apply_game_event_packet(&mut self, p: play::GameEvent) {
        match p.event {
            // NO_RESPAWN_BLOCK_AVAILABLE: ClientPacketListener.handleGameEvent
            // sends the player this system message.
            0 => self.system_message(sunrise_client::text::Text::translate(
                "block.minecraft.spawn.not_valid",
                &[],
            )),
            // LEVEL_CHUNKS_LOAD_START: the server has begun sending the level.
            13 => {
                if let Some(r) = self.remote.as_mut() {
                    r.level_load.loading_packets_received();
                }
            }
            // GUARDIAN_ELDER_EFFECT: the apparition, and the curse sound
            // unless the elder guardian is silent.
            10 => {
                self.particles.elder_guardian();
                if p.rounded_param() == 1
                    && let Some(at) = self.player.as_ref().map(|m| m.position)
                {
                    self.play_client_local_sound(
                        LevelSound::new(
                            "minecraft:entity.elder_guardian.curse",
                            [at.x, at.y, at.z],
                            1.0,
                            1.0,
                        )
                        .in_source(mod_api::SoundSource::Hostile),
                    );
                }
            }
            // WIN_GAME: the poem and credits, then PERFORM_RESPAWN.
            4 if self.screen != Screen::Win => {
                self.open_win_screen(true, super::super::win::Finish::RemoteRespawn);
            }
            // START_RAINING, STOP_RAINING, RAIN_LEVEL_CHANGE and THUNDER_LEVEL_CHANGE.
            event @ (1 | 2 | 7 | 8) => self.world.weather.game_event(event, p.param),
            3 => {
                self.set_mode_id(p.rounded_param());
                let mode = self.mode_id();
                if let Some(r) = self.remote.as_mut() {
                    r.game_mode = mode;
                }
                if self.spectator() {
                    self.reset_client_item_use();
                }
            }
            // IMMEDIATE_RESPAWN: LocalPlayer.setShowDeathScreen.
            11 => {
                if let Some(r) = self.remote.as_mut() {
                    r.show_death_screen = p.param == 0.0;
                }
            }
            // PLAY_ARROW_HIT_SOUND: ClientPacketListener.handleGameEvent
            // plays ARROW_HIT_PLAYER at player eye height.
            6 => {
                if let Some(at) = self.eye_position() {
                    self.play_client_local_sound(
                        LevelSound::new("minecraft:entity.arrow.hit_player", at, 0.18, 0.45)
                            .in_source(mod_api::SoundSource::Players),
                    );
                }
            }
            // PUFFER_FISH_STING: ClientPacketListener.handleGameEvent
            // plays PUFFER_FISH_STING at player position.
            9 => {
                if let Some(at) = self.player.as_ref().map(|m| m.position) {
                    self.play_client_local_sound(
                        LevelSound::new(
                            "minecraft:entity.puffer_fish.sting",
                            [at.x, at.y, at.z],
                            1.0,
                            1.0,
                        )
                        .in_source(mod_api::SoundSource::Neutral),
                    );
                }
            }
            // LIMITED_CRAFTING: LocalPlayer.setDoLimitedCrafting.
            12 => {
                if let Some(r) = self.remote.as_mut() {
                    r.do_limited_crafting = p.param == 1.0;
                }
            }
            _ => {}
        }
    }
}

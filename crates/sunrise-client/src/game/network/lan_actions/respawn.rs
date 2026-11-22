use super::*;
use crate::game::{commands::Target, server_travel::SpawnAction};

impl Game<'_> {
    pub(in crate::game) fn guest_respawn(&mut self, conn: u32) {
        self.after_commit(crate::game::transactions::After::PlayerRespawn(conn));
    }

    /// `handleClientCommand(PERFORM_RESPAWN)` calls PlayerList, not ServerPlayer.teleport.
    pub(in crate::game) fn commit_player_respawn(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(guest) = host.hub.guest(conn) else {
            return;
        };
        let end_conquered = host
            .play
            .get_mut(&conn)
            .is_some_and(|play| std::mem::take(&mut play.won_game));
        if end_conquered || (guest.dead && !guest.respawning) {
            self.queue_spawn_trip(
                conn,
                SpawnAction::Replace {
                    was_death: !end_conquered,
                },
            );
        }
    }

    /// Player.remove in the source level. The close hook still writes the old player's map.
    pub(in crate::game) fn remove_old_player_for_clone(&mut self, conn: u32, was_death: bool) {
        let Some(entity) = self
            .lan
            .as_ref()
            .and_then(|h| h.hub.guest(conn))
            .filter(|g| g.placed)
            .map(|g| g.entity_id as u32)
        else {
            return;
        };
        self.eject_player_riders(Some(entity));
        self.guest_close_menus(
            conn,
            if was_death {
                MenuRemoval::Killed
            } else {
                MenuRemoval::ChangedDimension
            },
        );
        if let Some(host) = self.lan.as_mut()
            && let Some(i) = host.hub.guests.connection_index(conn)
        {
            host.hub.remove_player_for_respawn(i);
            if let Some(map) = host.hub.guests.attachments_mut(conn) {
                map.retire();
                map.unbind_index();
            }
        }
        self.entity_section_forget(entity);
    }

    /// PlayerList.respawn, after its TeleportTransition and NeoForge's redirect are final.
    pub(in crate::game) fn place_cloned_player(
        &mut self,
        conn: u32,
        to: crate::game::respawn_position::Respawn,
        was_death: bool,
        copy_spawn: bool,
        missing: bool,
    ) {
        let Some(old) = self.command_guest(conn) else {
            return;
        };
        let old_spawn = old.spawn;
        let old_saved = old.respawn_elsewhere.clone();
        let old_config = old_saved.as_ref().and_then(RespawnConfig::from_saved);
        if let Some(host) = self.lan.as_mut()
            && let Some(play) = host.play.get_mut(&conn)
        {
            play.equipment_heard = None;
            play.inventory_heard = None;
            play.level_up_sound = Default::default();
        }
        let keep_inventory_rule = self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::KEEP_INVENTORY);
        let motion = self.motion;
        if !self.restore_cloned_player(
            conn,
            to.dim,
            was_death,
            |g| {
                if let Some(previous) = g.new_body(!was_death) {
                    let previous = *previous;
                    g.logs_kept = Some((previous.hurt_log, previous.attack_log));
                }
                // Even a credits return constructs a new entity. Keep its restored attributes
                // and health, not the old body's Entity.tickCount.
                g.carried_tick_count = None;
                // Entity's constructor starts it at the origin; snapTo follows clone and copy.
                g.place([0.0; 3]);
                (g.yaw, g.pitch) = (0.0, 0.0);
                g.spawn = None;
                g.respawn_elsewhere = None;
                if !was_death {
                    return;
                }
                g.dead = false;
                g.death_time = 0;
                g.food = 20;
                g.saturation = 5.0;
                g.health = g.respawn_health(motion);
                let mut xp = Experience {
                    level: g.xp_level,
                    progress: g.xp_progress,
                    total: g.xp_total,
                };
                let keep_inventory = keep_inventory_rule || g.game_mode == SPECTATOR;
                if !keep_inventory {
                    g.inventory.fill(None);
                    g.armor.fill(None);
                    g.offhand = None;
                    g.equipment = Default::default();
                    g.selected = 0;
                }
                xp.respawn(keep_inventory);
                g.effects = Default::default();
                (g.xp_level, g.xp_progress, g.xp_total) = (xp.level, xp.progress, xp.total);
            },
            |game| {
                if !copy_spawn {
                    return;
                }
                // NeoForge's setRespawnPosition event can veto the copy without
                // undoing a point already set by a clone listener.
                let legacy = old_config.is_none().then_some(old_spawn).flatten();
                let config = old_config.or_else(|| {
                    legacy.map(|at| {
                        RespawnConfig::forced(
                            Dim::Overworld,
                            at.map(|c| c.floor() as i32),
                            0.0,
                            0.0,
                        )
                    })
                });
                let accepted = game.as_guest(conn, |g| {
                    if !g.set_respawn_point(config) {
                        return false;
                    }
                    // Keep the save's legacy point as a literal position, not a block lookup.
                    if let Some([x, y, z]) = legacy {
                        g.spawn = Some(Vec3::new(x, y, z));
                        g.respawn_config = None;
                    }
                    true
                });
                if accepted == Some(true)
                    && config.is_none()
                    && let Some(host) = game.lan.as_mut()
                {
                    // A config in a missing dimension is preserved in the save.
                    host.hub
                        .update_guest(conn, |g| g.respawn_elsewhere = old_saved);
                }
            },
        ) {
            return;
        }
        let Some(i) = self
            .lan
            .as_ref()
            .and_then(|h| h.hub.guests.connection_index(conn))
        else {
            return;
        };
        let clock_rate = self.network_clock_rate();
        let world_spawn = self.world_spawn_position();
        let ctx = TickCtx {
            world: &mut self.world,
            entities: &[],
            next_entity: &mut self.next_entity,
            spawn: world_spawn,
            spawn_angle: self.level.world_spawn_angle,
            spawn_dimension: crate::game::Dim::of_world_spawn(&self.level).id(),
            sea_level: self.end.locator().map_or(63, |terrain| terrain.sea_level()),
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
            streaming: Some(&self.streaming),
        };
        if let Some(host) = self.lan.as_mut() {
            if missing {
                host.hub.missing_respawn_block(conn);
            }
            host.hub.respawn(
                i,
                [to.at.x, to.at.y, to.at.z],
                Some([to.yaw, to.pitch]),
                !was_death,
                &ctx,
            );
        }
        self.bind_player_attachments(conn, false);
        if mod_api::__private::listened(mod_api::HookId::PlayerRespawned)
            && let Some(guest) = self.command_guest(conn)
        {
            let (uuid, name) = (guest.uuid, guest.name.clone());
            self.as_guest(conn, |g| g.native_player_respawned(uuid, &name, !was_death));
        }
        if !was_death {
            self.as_guest(conn, |g| {
                g.trigger_changed_dimension(crate::game::Dim::End, crate::game::Dim::Overworld)
            });
        } else {
            if let Some(config) = self
                .command_guest(conn)
                .and_then(|g| g.respawn_elsewhere.as_ref())
                .and_then(RespawnConfig::from_saved)
            {
                // PlayerList.respawn uses Level.getBlockState, which loads the saved point's
                // column to FULL (unless it is outside the level's addressable bounds).
                let placement = self.dim;
                let anchor = match self.enter_level(config.dim) {
                    Ok(()) => {
                        let [x, y, z] = config.pos;
                        let chunk = (x >> 4, z >> 4);
                        // ChunkPyramid.MAX_CHUNK_COORDINATE_VALUE: the packed BlockPos range,
                        // less the player view radius and FULL's 12 accumulated dependency rings.
                        const MAX_CHUNK: i32 = (((1 << 25) - 1) >> 4) - 2 * (32 + 12 + 1);
                        if !self.world.shape.contains_y(y)
                            || chunk.0.abs().max(chunk.1.abs()) > MAX_CHUNK
                        {
                            false
                        } else if self.streaming.load_full_column(&mut self.world, chunk) {
                            self.is_anchor(self.world.block(x, y, z))
                        } else {
                            log::error!(
                                "could not load the respawn sound's saved point at {config:?}"
                            );
                            false
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "could not open {} for the respawn sound: {e}",
                            config.dim.id()
                        );
                        false
                    }
                };
                if let Err(e) = self.enter_level(placement) {
                    log::error!("could not reopen {}: {e}", placement.id());
                    return;
                }
                // Send only to this connection, using the placement level's RNG.
                if anchor {
                    let seed = self.world.random.next_long();
                    self.play_sound_to(
                        Target::Guest(conn),
                        "minecraft:block.respawn_anchor.deplete",
                        config.pos.map(f64::from),
                        1.0,
                        1.0,
                        mod_api::SoundSource::Blocks.index(),
                        seed,
                    );
                }
            }
            let Some(guest) = self.command_guest(conn) else {
                return;
            };
            let from = crate::game::game_mode::mode_from_id(guest.game_mode);
            if !self.level.hardcore || from == GameMode::Spectator {
                return;
            }
            let (uuid, name) = (guest.uuid, guest.name.clone());
            let (mut cancel, mut to) = (false, GameMode::Spectator);
            self.native_player_game_mode_changed(uuid, &name, from, &mut to, &mut cancel);
            if !cancel
                && to != from
                && let Some(host) = self.lan.as_mut()
            {
                host.hub
                    .set_game_mode(conn, to.id(), &self.world, Some(self.collision));
            }
        }
    }

    /// The combat history follows the body into and out of an acting turn.
    pub(in crate::game) fn guest_combat_record(
        &self,
        conn: u32,
    ) -> Option<&crate::game::combat::CombatTracker> {
        if self.lan_acting == Some(conn) {
            Some(&self.combat)
        } else {
            self.guest_combat.get(&conn)
        }
    }

    /// Run part of `ServerPlayer.die` on the victim's body. A hit that already
    /// has the body installed must not try to enter the same guest again.
    fn with_dying_guest(&mut self, conn: u32, f: impl FnOnce(&mut Self)) {
        if self.lan_acting == Some(conn) {
            f(self);
        } else {
            self.as_guest(conn, f);
        }
    }

    /// `ServerPlayer.die` for a guest whose health has run out: the death
    /// message, the loot and experience, the death screen.
    pub(in crate::game) fn guest_die(&mut self, gi: usize, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let guest = &host.hub.guests[gi];
        let id = guest.entity_id;
        let acting = self.lan_acting == Some(conn);
        let dying = |game: &Self, guest: &sunrise_client::lan::guest::Guest| {
            !guest.dead
                && if acting {
                    game.player
                        .as_ref()
                        .is_some_and(|body| body.health.health() <= 0.0)
                } else {
                    guest.health <= 0.0
                }
        };
        if !dying(self, guest) {
            return;
        }
        // The killing blow, for the death hooks.
        let blow = (!mod_api::__private::mods().is_empty()).then(|| self.guest_damage_source(conn));
        if self.guest_death_canceled(gi, conn) {
            return;
        }
        // A dead player carries no mob a mod seated on it.
        self.eject_player_riders(Some(id as u32));
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let g = &host.hub.guests[gi];
        if dying(self, g) {
            let (uuid, name) = (g.uuid, g.name.clone());
            let here = host.hub.here(g);
            let (at, eye, xp, spectator) = if acting {
                let Some(body) = self.player.as_ref() else {
                    return;
                };
                (
                    [body.position.x, body.position.y, body.position.z],
                    f64::from(body.dimensions.eye_height),
                    self.experience,
                    self.spectator(),
                )
            } else {
                (
                    g.position,
                    g.eye_height(),
                    Experience {
                        level: g.xp_level,
                        progress: g.xp_progress,
                        total: g.xp_total,
                    },
                    g.game_mode == SPECTATOR,
                )
            };
            let last_death = Some((g.dimension.clone(), at.map(|v| v.floor() as i32)));
            let reward = xp.death_reward(
                self.ui
                    .rules
                    .bool_at(sunrise_client::gamerules::vanilla::KEEP_INVENTORY)
                    || spectator,
            );
            let drops = (!spectator).then(|| [at[0], at[1] + eye - f64::from(0.3f32), at[2]]);
            let cause = if acting {
                self.mining = None;
                self.destroy_delay = 0;
                self.using = None;
                self.hook.discard();
                self.level.last_death = last_death;
                self.ui.death_cause.take()
            } else {
                self.guest_forget_digging(conn);
                self.lan.as_mut().and_then(|host| {
                    host.hub.guests.update(gi, |g| g.last_death = last_death);
                    host.play.get_mut(&conn).and_then(|play| {
                        play.hook.discard();
                        play.death_cause.take()
                    })
                })
            };
            if let Some(host) = self.lan.as_mut() {
                host.hub.guests.update(gi, |g| g.dead = true);
            }
            // ServerPlayer.die opens with `gameEvent(ENTITY_DIE)` at the player:
            // sculk sensors and wardens hear the death.
            if here {
                self.with_dying_guest(conn, |game| {
                    let ctx = game.player_context(None);
                    game.post_game_event("minecraft:entity_die", at, ctx);
                });
            }
            // `getKillCredit`, as the death finds it.
            let killer = self.guest_kill_credit_type(conn);
            self.native_player_died(uuid, cause.as_deref());
            // ServerPlayer.die: the neutral mobs near a guest in this level
            // forget their anger at them.
            if here {
                self.tell_neutral_mobs_player_died(at, Some(conn));
            }
            // The death's last blow, read before the death message empties the
            // combat record: ServerPlayer.die's triggers read it from a record
            // that CombatTracker.recheckStatus clears only at the end of die.
            let killing = self
                .guest_combat_record(conn)
                .and_then(|c| c.killing_blow());
            // ServerPlayer.die: with showDeathMessages, the message goes to
            // the dying player's death screen and to everyone the team lets
            // see it; without it, the death screen gets an empty one.
            let message = if self
                .ui
                .rules
                .bool_at(sunrise_client::gamerules::vanilla::SHOW_DEATH_MESSAGES)
            {
                let me = self.player_text(&name, uuid);
                let text = match cause {
                    Some(key) => sunrise_client::text::Text::translate(&key, &[me]),
                    None if at[1] < self.world.below_world_y() => {
                        sunrise_client::text::Text::translate("death.attack.outOfWorld", &[me])
                    }
                    None => self.guest_death_message(conn, me),
                };
                let message = sunrise_client::lan::data::component_nbt(&text.component());
                let board = &self.score.board;
                let Some(host) = self.lan.as_ref() else {
                    return;
                };
                host.hub.system_message_to(message.clone(), |other| {
                    sunrise_client::scoreboard::death_message_reaches(board, &name, other)
                });
                if self.is_dedicated_server() {
                    crate::game::dedicated::events::record(
                        "death",
                        &name,
                        uuid,
                        serde_json::json!({ "message": text.plain() }),
                    );
                    // Only a message for everyone is logged (`broadcastSystemMessage`).
                    if board.players_team(&name).is_none_or(|t| {
                        t.death_message_visibility == sunrise_client::scoreboard::Visibility::Always
                    }) {
                        crate::game::dedicated::log(
                            "INFO",
                            &format!("System chat: {}", text.plain()),
                        );
                    }
                } else if self.player.is_some()
                    && sunrise_client::scoreboard::death_message_reaches(
                        board,
                        &name,
                        crate::game::chat::PLAYER_NAME,
                    )
                {
                    self.system_message(text);
                }
                message
            } else {
                literal("")
            };
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            host.hub.send_to(
                conn,
                &C::PlayerCombatKill(play::PlayerCombatKill { id, message }),
            );
            // LivingEntity.die: the death animation, the dying player included.
            host.hub.entity_event(id, 3);
            let recently_hit = killing.is_some()
                && self
                    .guest_combat_record(conn)
                    .is_some_and(|c| c.recently_hit_by_player(self.world.game_time));
            let dropping = crate::game::creatures::DyingDrops {
                entity: id as u32,
                kind: "minecraft:player",
                player: Some(uuid),
                source: killing.as_ref().map(|(damage_type, attacker)| {
                    crate::game::creatures::death_hurt_source(damage_type, attacker.unwrap_or(0))
                }),
                recently_hit,
            };
            // LivingEntity.die: the death is heard, and a sculk catalyst
            // nearby takes the experience (before the loot is dropped), as it
            // does for a mob.
            let mut catalyst_xp = 0;
            let consumed = self.catalyst_takes_experience(at, |_| {
                catalyst_xp = reward;
                catalyst_xp
            });
            // CatalystListener: with experience to take, the player who last
            // hurt them gets KILL_MOB_NEAR_SCULK_CATALYST.
            if consumed && catalyst_xp > 0 {
                self.guest_catalyst_trigger(gi, conn, at);
            }
            // ServerPlayer.die: the drops, the deathCount objectives, the
            // experience (`dropAllDeathLoot`), then a kill credit's `killed_by`,
            // awardKillScore and wither rose, then `deaths` and the two timers.
            self.with_dying_guest(conn, |game| {
                if let Some(eyes) = drops {
                    game.drop_inventory(eyes, &dropping);
                }
                game.score_player_died();
                if let Some(kind) = &killer {
                    game.award_stat("killed_by", kind, 1);
                }
            });
            // ServerPlayer.die: a spectator never reaches dropAllDeathLoot. The
            // experience the catalyst took does not drop (`Player.skipDropExperience`),
            // and `dropAllDeathLoot` drops it before the rose.
            if drops.is_some() {
                // LivingEntity.dropExperience: Player.isAlwaysExperienceDropper.
                let now = self.world.game_time;
                let attacker = if mod_api::__private::mods().is_empty() {
                    None
                } else {
                    let credit = if acting {
                        self.combat.take_hurt_by_player(now)
                    } else {
                        self.guest_combat
                            .get_mut(&conn)
                            .and_then(|c| c.take_hurt_by_player(now))
                    };
                    credit.and_then(|c| self.resolve_player_credit(c).map(|_| c.0))
                };
                let reward = self.living_experience_drop(
                    id as u32,
                    "minecraft:player",
                    attacker,
                    reward as i32,
                );
                if !consumed {
                    self.spawn_orbs(at, reward as u32);
                }
            }
            self.guest_killed_player_trigger(conn, at, killing);
            self.guest_award_kill_score(conn, at);
            self.with_dying_guest(conn, |game| {
                game.award_custom_stat("deaths", 1);
                game.stats.reset_death_timers();
            });
            // `ServerPlayer.die`'s `broadcastEntityEvent`, after the message,
            // the loot and the experience: Fabric's AFTER_DEATH. The blow is
            // the one `living_death` heard, so a death it cancelled returns
            // above and is never told here.
            if let Some((damage_type, attacker)) = &blow
                && mod_api::__private::listened(mod_api::HookId::LivingDeathPost)
            {
                self.native_living_death_post(id as u32, damage_type, *attacker);
            }
        }
    }

    /// `SculkCatalystBlockEntity.CatalystListener`'s trigger on a player's
    /// death: for the player the dying guest's combat record credits
    /// (`LivingEntity.getKillCredit`), with the death's `killing_blow`.
    pub(super) fn guest_catalyst_trigger(&mut self, gi: usize, conn: u32, at: [f64; 3]) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let now = self.world.game_time;
        let combat = self.guest_combat_record(conn);
        let player_credit = combat.and_then(|c| c.player_credit(now));
        let who = if let Some(credit) = player_credit {
            self.resolve_player_credit(credit)
        } else {
            combat.and_then(|c| c.kill_credit(now)).and_then(|id| {
                if self.lan_acting.is_none()
                    && self
                        .player
                        .as_ref()
                        .is_some_and(|p| p.entity_id as u32 == id)
                {
                    Some(None)
                } else {
                    host.hub
                        .guests
                        .iter()
                        .find(|g| g.entity_id as u32 == id)
                        .map(|g| Some(g.conn))
                }
            })
        };
        let Some(who) = who else { return };
        let (damage, attacker) = combat
            .and_then(|c| c.killing_blow())
            .unwrap_or_else(|| ("minecraft:player_attack".into(), None));
        let g = &host.hub.guests[gi];
        let view = sunrise_client::advancements::EntityView {
            kind: "minecraft:player".into(),
            position: at,
            sneaking: g.sneaking,
            input_sneak: g.input & 32 != 0,
            gamemode: Some(
                match g.game_mode {
                    1 => "creative",
                    2 => "adventure",
                    3 => "spectator",
                    _ => "survival",
                }
                .into(),
            ),
            ..Default::default()
        };
        // The damage's causing entity: the player in place, another guest, or
        // a mob (`DamageSource.getEntity`).
        let source = attacker.and_then(|id| {
            if self.lan_acting.is_none()
                && self
                    .player
                    .as_ref()
                    .is_some_and(|p| p.entity_id as u32 == id)
            {
                Some(self.player_entity_for_any(&["minecraft:kill_mob_near_sculk_catalyst"]))
            } else {
                host.hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id as u32 == id)
                    .map(|g| sunrise_client::advancements::EntityView {
                        kind: "minecraft:player".into(),
                        position: g.position,
                        sneaking: g.sneaking,
                        input_sneak: g.input & 32 != 0,
                        ..Default::default()
                    })
                    .or_else(|| {
                        self.mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| self.mob_entity(m))
                    })
            }
        });
        let projectile = match damage.as_str() {
            "minecraft:arrow" => Some("minecraft:arrow"),
            "minecraft:fireball" | "minecraft:small_fireball" => Some("minecraft:fireball"),
            "minecraft:wither_skull" => Some("minecraft:wither_skull"),
            "minecraft:fireworks" => Some("minecraft:firework_rocket"),
            _ => None,
        };
        let blow = sunrise_client::advancements::DamageView {
            kind: damage,
            // DamageSource.isDirect: the causing entity is the direct one.
            is_direct: projectile.is_none(),
            direct: match projectile {
                Some(kind) => Some(sunrise_client::advancements::EntityView {
                    kind: kind.into(),
                    position: at,
                    ..Default::default()
                }),
                None => source.clone(),
            },
            source,
            ..Default::default()
        };
        // CatalystListener: trigger(player, killedEntity, damageSource).
        match who {
            None => {
                self.trigger_killed_as("minecraft:kill_mob_near_sculk_catalyst", &view, Some(&blow))
            }
            Some(kconn) => {
                self.as_guest(kconn, |g| {
                    g.trigger_killed_as(
                        "minecraft:kill_mob_near_sculk_catalyst",
                        &view,
                        Some(&blow),
                    );
                });
            }
        }
    }

    /// `ServerPlayer.die` → `Entity.awardKillScore` on the entity the dying
    /// guest's combat record credits (`LivingEntity.getKillCredit`): the
    /// guest's `entity_killed_player`, with that entity and the death's
    /// `killing_blow`. Nothing credited, nothing fires.
    pub(super) fn guest_killed_player_trigger(
        &mut self,
        conn: u32,
        at: [f64; 3],
        blow: Option<(String, Option<u32>)>,
    ) {
        const TRIGGER: &str = "minecraft:entity_killed_player";
        let now = self.world.game_time;
        let Some(id) = self
            .guest_combat_record(conn)
            .and_then(|c| c.kill_credit(now))
        else {
            return;
        };
        let Some(killer) = self.guest_entity_view(id, TRIGGER) else {
            return;
        };
        let blow = self.guest_killing_blow(blow, at, TRIGGER);
        self.with_dying_guest(conn, |g| {
            g.trigger_killed_as(TRIGGER, &killer, blow.as_ref());
        });
    }

    /// The type of the entity a dying guest's combat record credits with the
    /// kill (`LivingEntity.getKillCredit`): a player (the one in place or a
    /// guest), a mob (one gone already credits nobody, as vanilla's reference
    /// no longer resolves), a mob whose own blast this tick killed the guest,
    /// or the ender dragon.
    fn guest_kill_credit_type(&self, conn: u32) -> Option<String> {
        let id = self
            .guest_combat_record(conn)?
            .kill_credit(self.world.game_time)?;
        if self.combat_player(id).is_some() {
            return Some("minecraft:player".into());
        }
        if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            return Some(m.kind.id.clone());
        }
        // A mob that blew up this tick has left the list, but vanilla's player
        // dies inside that blast, before the source is discarded.
        if let Some((_, kind)) = self.combat.exploded_kinds.iter().rfind(|(m, _)| *m == id) {
            return Some(kind.clone());
        }
        self.dragon
            .as_ref()
            .filter(|d| d.id == id)
            .map(|_| "minecraft:ender_dragon".into())
    }

    /// The dying guest's last blow (`LivingEntity.getLastDamageSource`), its
    /// damage type and causing entity as the combat record held them, as a
    /// damage source: the projectile as the direct entity.
    pub(super) fn guest_killing_blow(
        &self,
        blow: Option<(String, Option<u32>)>,
        at: [f64; 3],
        trigger: &str,
    ) -> Option<sunrise_client::advancements::DamageView> {
        let (damage, attacker) = blow?;
        // The damage's causing entity: the player in place, another guest, or
        // a mob (`DamageSource.getEntity`).
        let source = attacker.and_then(|id| self.guest_entity_view(id, trigger));
        Some(sunrise_client::advancements::recorded_blow(
            damage, source, at,
        ))
    }

    /// The entity behind a combat record's id, as `trigger`'s predicates see
    /// it: an installed, displaced or parked player, or a mob.
    pub(super) fn guest_entity_view(
        &self,
        id: u32,
        trigger: &str,
    ) -> Option<sunrise_client::advancements::EntityView> {
        use sunrise_client::advancements::EntityView;
        match self.combat_player(id) {
            // `player_entity_for` takes `&mut self` (the compound for `nbt`);
            // this view of the killer has the player's records without it.
            Some(CombatPlayer::InPlace) => Some(self.player_entity_for_any(&[trigger])),
            Some(CombatPlayer::Displaced(depth, body)) => {
                self.displaced_player_entity_for(&self.lan_displaced_players[depth], body, trigger)
            }
            Some(CombatPlayer::Parked(g)) => Some(EntityView {
                kind: "minecraft:player".into(),
                position: g.position,
                sneaking: g.sneaking,
                input_sneak: g.input & 32 != 0,
                ..Default::default()
            }),
            None => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| self.mob_entity(m)),
        }
    }

    /// The scalar death-tail/death-message source. `living_death` uses the
    /// full accepted hit retained on the body instead.
    pub(super) fn guest_damage_source(&self, conn: u32) -> (String, u32) {
        let cause = if self.lan_acting == Some(conn) {
            self.ui.death_cause.as_deref()
        } else {
            self.lan
                .as_ref()
                .and_then(|host| host.play.get(&conn))
                .and_then(|p| p.death_cause.as_deref())
        };
        let (damage_type, attacker) = match cause {
            Some("death.attack.genericKill") => ("minecraft:generic_kill", 0),
            _ => self
                .guest_combat_record(conn)
                .and_then(|tracker| tracker.last_blow())
                .unwrap_or(("minecraft:generic", 0)),
        };
        (damage_type.to_owned(), attacker)
    }

    /// `ServerPlayer.die`'s message from a guest's own `CombatTracker`, which
    /// sits in `guest_combat` between turns.
    pub(super) fn guest_death_message(
        &mut self,
        conn: u32,
        me: sunrise_client::text::Text,
    ) -> sunrise_client::text::Text {
        if self.lan_acting == Some(conn) {
            return self.combat_death_message_for(me);
        }
        let mut tracker = self.guest_combat.remove(&conn).unwrap_or_default();
        std::mem::swap(&mut self.combat, &mut tracker);
        let text = self.combat_death_message_for(me);
        std::mem::swap(&mut self.combat, &mut tracker);
        self.guest_combat.insert(conn, tracker);
        text
    }
}
